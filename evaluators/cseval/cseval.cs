//
// cseval.cs: evaluate code from evalbot
// repl.cs: Support for using the compiler in interactive mode (read-eval-print loop)
//
// Authors:
//   Miguel de Icaza (miguel@gnome.org)
//
// Dual licensed under the terms of the MIT X11 or GNU GPL
//
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

namespace CSEval {

    public class Driver {
        private static readonly StringWriter sw = new StringWriter();

        public static StringWriter Output => sw;

        static int Main(string[] args) {
            var cmd = new CommandLineParser(Console.Error);

            // Enable unsafe code by default
            var settings = new CompilerSettings() {
                Unsafe = true,
            };

            if (!cmd.ParseArguments(settings, args))
                return 1;

            Console.SetOut(Output);
            Console.SetError(Output);
            Console.SetIn(new StringReader(""));

            ReportPrinter printer = new ConsoleReportPrinter();

            Func<Evaluator> newEval = () => {
                Evaluator eval = new Evaluator(new CompilerContext(settings, printer)) {
                    InteractiveBaseClass = typeof(InteractiveBase),
                    DescribeTypeExpressions = true,
                    WaitOnTask = true
                };
                return eval;
            };

            CSharpShell shell = new CSharpShell(newEval, Console.OpenStandardInput(), Console.OpenStandardOutput());
            return shell.Run();
        }
    }

    public class CSharpShell {
        private readonly Func<Evaluator> newEval;
        private readonly Dictionary<string, Evaluator> evaluators;
        private readonly Stream input;
        private readonly Stream output;

        public CSharpShell(Func<Evaluator> newEval, Stream input, Stream output) {
            this.newEval = newEval;
            this.input = input;
            this.output = output;
            this.evaluators = new Dictionary<String, Evaluator>();
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

        private void ReturnWork(string result) {
            output.WriteLengthUTF8(result);
            output.Flush();
        }

        public int Run() {
            Dictionary<string, string> exprs = new Dictionary<string, string>();
            while (true) {
                int timeout = input.ReadInt32();
                int keylen = input.ReadInt32();
                int codelen = input.ReadInt32();
                string key = input.ReadUTF8(keylen);
                string work = input.ReadUTF8(codelen).Trim();

                if (work == "") {
                    ReturnWork("");
                    continue;
                }

                string output = null;
                string evopt =
                    Evaluate(key,
                        !exprs.ContainsKey(key) ? work : exprs[key] + "\n" + work,
                        timeout, ref output);

                if (output != null || evopt == null) { // exception or result
                    ReturnWork(output ?? "");
                } else if (output == null && evopt != null) { // continuation
                    ReturnWork("(continue...)");
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

            Driver.Output.GetStringBuilder().Clear();

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
                            PrettyPrinter.PrettyPrint(Driver.Output, result);
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
                Driver.Output.WriteLine(e.ToString());
                output = Driver.Output.ToString();
                return null;
            }
            if (Driver.Output.GetStringBuilder().Length > 0) {
                output = Driver.Output.ToString();
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
