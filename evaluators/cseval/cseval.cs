//
// cseval.cs: evaluate code from evalbot
// repl.cs: Support for using the compiler in interactive mode (read-eval-print loop)
//
// Authors:
//   Miguel de Icaza (miguel@gnome.org)
//
// Dual licensed under the terms of the MIT X11 or GNU GPL
//
// Copyright 2016 angelsl
// Copyright 2001, 2002, 2003 Ximian, Inc (http://www.ximian.com)
// Copyright 2004, 2005, 2006, 2007, 2008 Novell, Inc
// Copyright 2011-2013 Xamarin Inc
//
//
// TODO:
//   Do not print results in Evaluate, do that elsewhere in preparation for Eval refactoring.
//   Driver.PartialReset should not reset the coretypes, nor the optional types, to avoid
//      computing that on every call.
//
using System;
using System.IO;
using System.Text;
using System.Globalization;
using System.Collections;
using System.Reflection;
using System.Reflection.Emit;
using System.Threading;
using System.Threading.Tasks;
using System.Net;
using System.Net.Sockets;
using System.Collections.Generic;

using Mono.CSharp;
using Mono.Unix;
using Mono.Unix.Native;

namespace CSEval {
    public class Driver {
        static void Main(string[] args) {
            CompilerSettings settings = new CompilerSettings() { Unsafe = true };
            ConsoleReportPrinter printer = new ConsoleReportPrinter();

            CSharpShell shell = new CSharpShell(() => new Evaluator(new CompilerContext(settings, printer)) {
                InteractiveBaseClass = typeof(InteractiveBase),
                DescribeTypeExpressions = true,
                WaitOnTask = true
            });

            try {
                Syscall.unlink(args[0]);
            } catch {}
            UnixListener sock = new UnixListener(args[0]);
            sock.Start();
            Syscall.chmod(args[0], FilePermissions.ACCESSPERMS);

            while (true) {
                NetworkStream s = new NetworkStream(sock.AcceptSocket(), true);
                Task.Run(() => {
                    try {
                        shell.ProcessConnection(s);
                    } finally {
                        s.Dispose();
                    }
                });
            }
        }
    }

    public class CSharpShell {
        private static readonly StringWriter StdOut = new StringWriter();

        private readonly Func<Evaluator> newEval;
        private readonly Dictionary<string, Evaluator> evaluators;
        private readonly Dictionary<string, string> exprs;

        public CSharpShell(Func<Evaluator> newEval) {
            Console.SetOut(StdOut);
            Console.SetError(StdOut);
            Console.SetIn(new StringReader(""));

            this.newEval = newEval;
            this.evaluators = new Dictionary<String, Evaluator>();
            this.exprs = new Dictionary<string, string>();
        }

        private Evaluator GetEvaluator(string key) {
            if (evaluators.ContainsKey(key)) {
                return evaluators[key];
            } else {
                Evaluator ev = newEval();
                evaluators[key] = ev;
                string nul = null;
                Evaluate(key, "using System; using System.Linq; using System.Collections.Generic; using System.Collections;", 0, ref nul);
                return ev;
            }
        }

        private void ReturnWork(string result, Stream conn) {
            conn.WriteLengthUTF8(result);
            conn.Flush();
        }

        public void ProcessConnection(Stream conn) {
            int timeout = conn.ReadInt32();
            int keylen = conn.ReadInt32();
            int codelen = conn.ReadInt32();
            string key = conn.ReadUTF8(keylen);
            string work = conn.ReadUTF8(codelen).Trim();

            if (work == "") {
                ReturnWork("", conn);
                return;
            }

            lock (exprs) {
                string output = null;
                string evopt =
                    Evaluate(key,
                        !exprs.ContainsKey(key) ? work : exprs[key] + "\n" + work,
                        timeout, ref output);

                if (output != null || evopt == null) { // exception or result
                    ReturnWork(output ?? "", conn);
                } else if (output == null && evopt != null) { // continuation
                    ReturnWork("(continue...)", conn);
                }

                exprs[key] = evopt;
            }

        }

        private Tuple<string, bool, object> EvaluateHelper(Evaluator ev, string input, CancellationToken canceller) {
            using (canceller.Register(Thread.CurrentThread.Abort)) {
                bool result_set;
                object result;

                input = ev.Evaluate(input, out result, out result_set);
                return Tuple.Create(input, result_set, result);
            }
        }

        private string Evaluate(string key, string input, int timeout, ref string output) {
            bool result_set;
            object result;

            StdOut.GetStringBuilder().Clear();

            CancellationTokenSource canceller = new CancellationTokenSource();

            try {
                Task<Tuple<string, bool, object>> t = Task.Run(() => EvaluateHelper(GetEvaluator(key), input, canceller.Token), canceller.Token);
                if (timeout != 0) canceller.CancelAfter(timeout);
                if (timeout == 0 || t.Wait(timeout)) {
                    Tuple<string, bool, object> resultTuple = t.Result;
                    if (resultTuple != null) {
                        input = resultTuple.Item1;
                        result_set = resultTuple.Item2;
                        result = resultTuple.Item3;
                        if (result_set) {
                            PrettyPrinter.PrettyPrint(StdOut, result);
                        }
                    } else {
                        output = "(timed out... probably?)";
                        return null;
                    }
                } else {
                    output = "(timed out)";
                    return null;
                }
            } catch (Exception e) {
                StdOut.WriteLine(e.ToString());
                output = StdOut.ToString();
                return null;
            }
            if (StdOut.GetStringBuilder().Length > 0) {
                output = StdOut.ToString();
            }
            return input;
        }
    }

    internal static class StreamHelper {
        public unsafe static int ReadInt32(this Stream s) {
            byte[] bytes = new byte[4];
            s.Read(bytes, 0, 4);
            fixed (byte* intP = bytes) {
                return *(int*)intP;
            }
        }

        public static string ReadUTF8(this Stream s, int l) {
            byte[] bytes = new byte[l];
            s.Read(bytes, 0, l);
            try {
                return Encoding.UTF8.GetString(bytes);
            } catch {
                return ""; // blah.
            }
        }

        public static string ReadLengthUTF8(this Stream s) {
            return s.ReadUTF8(s.ReadInt32());
        }

        public unsafe static void WriteLengthUTF8(this Stream s, string d) {
            if (d == null) {
                d = "";
            }

            byte[] strBytes = Encoding.UTF8.GetBytes(d);
            byte[] len = new byte[4];

            fixed(byte* lenCP = len) {
                int* lenP = (int*)lenCP;
                *lenP = strBytes.Length;
            }

            s.Write(len, 0, 4);
            s.Write(strBytes, 0, strBytes.Length);
        }
    }

    internal static class PrettyPrinter {
        private static void p(TextWriter output, string s) {
            output.Write(s);
        }

        private static string EscapeString(string s) {
            return s.Replace("\"", "\\\"");
        }

        private static void EscapeChar(TextWriter output, char c) {
            if (c == '\'') {
                output.Write("'\\''");
                return;
            }
            if (c > 32) {
                output.Write("'{0}'", c);
                return;
            }
            switch (c) {
            case '\a':
                output.Write("'\\a'");
                break;

            case '\b':
                output.Write("'\\b'");
                break;

            case '\n':
                output.Write("'\\n'");
                break;

            case '\v':
                output.Write("'\\v'");
                break;

            case '\r':
                output.Write("'\\r'");
                break;

            case '\f':
                output.Write("'\\f'");
                break;

            case '\t':
                output.Write("'\\t");
                break;

            default:
                output.Write("'\\x{0:x}", (int)c);
                break;
            }
        }

        // Some types (System.Json.JsonPrimitive) implement
        // IEnumerator and yet, throw an exception when we
        // try to use them, helper function to check for that
        // condition
        private static bool WorksAsEnumerable(object obj) {
            IEnumerable enumerable = obj as IEnumerable;
            if (enumerable != null) {
                try {
                    enumerable.GetEnumerator();
                    return true;
                } catch {
                    // nothing, we return false below
                }
            }
            return false;
        }

        public static void PrettyPrint(TextWriter output, object result) {
            if (result == null) {
                p(output, "null");
                return;
            }

            if (result is Array) {
                Array a = (Array)result;

                p(output, "{ ");
                int top = a.GetUpperBound(0);
                for (int i = a.GetLowerBound(0); i <= top; i++) {
                    PrettyPrint(output, a.GetValue(i));
                    if (i != top)
                        p(output, ", ");
                }
                p(output, " }");
            } else if (result is bool) {
                if ((bool)result)
                    p(output, "true");
                else
                    p(output, "false");
            } else if (result is string) {
                p(output, String.Format("\"{0}\"", EscapeString((string)result)));
            } else if (result is IDictionary) {
                IDictionary dict = (IDictionary)result;
                int top = dict.Count, count = 0;

                p(output, "{");
                foreach (DictionaryEntry entry in dict) {
                    count++;
                    p(output, "{ ");
                    PrettyPrint(output, entry.Key);
                    p(output, ", ");
                    PrettyPrint(output, entry.Value);
                    if (count != top)
                        p(output, " }, ");
                    else
                        p(output, " }");
                }
                p(output, "}");
            } else if (WorksAsEnumerable(result)) {
                int i = 0;
                p(output, "{ ");
                foreach (object item in (IEnumerable) result) {
                    if (i++ != 0)
                        p(output, ", ");

                    PrettyPrint(output, item);
                }
                p(output, " }");
            } else if (result is char) {
                EscapeChar(output, (char)result);
            } else {
                p(output, result.ToString());
            }
        }
    }
}
