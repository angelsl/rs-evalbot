using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.FSharp.Compiler.Interactive;
using Microsoft.FSharp.Core;

namespace FSEval {
    public static class Program {
        private static readonly StringReader DummyInput = new StringReader("");
        private static readonly Stream Input = Console.OpenStandardInput();
        private static readonly Stream Output = Console.OpenStandardOutput();
        private static readonly StringWriter EvalOutput = new StringWriter();
        private static readonly Dictionary<string, Shell.FsiEvaluationSession> _evaluators = new Dictionary<string, Shell.FsiEvaluationSession>();
        private static void Main(string[] args) {
            Console.SetOut(EvalOutput);
            Console.SetError(EvalOutput);
            Console.SetIn(DummyInput);
            Run();
        }

        private static Shell.FsiEvaluationSession GetEvaluator(string key) {
            if (_evaluators.ContainsKey(key)) {
                return _evaluators[key];
            } else {
                Shell.FsiEvaluationSession ev = Shell.FsiEvaluationSession.Create(
                    Shell.FsiEvaluationSession.GetDefaultConfiguration(),
                    new[] { "fsi", "--noninteractive" },
                    DummyInput, EvalOutput, EvalOutput, new FSharpOption<bool>(true));
                _evaluators[key] = ev;
                return ev;
            }
        }

        private static void ReturnWork(string result) {
            Output.WriteLengthUTF8(result);
            Output.Flush();
        }

        private static void Run() {
            while (true) {
                int timeout = Input.ReadInt32();
                int keylen = Input.ReadInt32();
                int codelen = Input.ReadInt32();
                string key = Input.ReadUTF8(keylen);
                string work = Input.ReadUTF8(codelen).Trim();

                if (work == "") {
                    ReturnWork("");
                    continue;
                }

                ReturnWork(Evaluate(key, work, timeout) ?? "");
            }
        }

        private static void EvaluateHelper(Shell.FsiEvaluationSession ev, string input, CancellationToken canceller) {
            using (canceller.Register(Thread.CurrentThread.Abort)) {
                try {
                    ev.EvalInteraction(input);
                } catch (Exception e) {
                    EvalOutput.WriteLine(e.InnerException ?? e);
                }
            }
        }

        private static string Evaluate(string key, string input, int timeout) {
            Shell.FsiEvaluationSession ev = GetEvaluator(key);
            EvalOutput.GetStringBuilder().Clear();
            CancellationTokenSource canceller = new CancellationTokenSource();
            try {
                Task t = Task.Run(() => EvaluateHelper(ev, input, canceller.Token), canceller.Token);
                if (timeout != 0) {
                    canceller.CancelAfter(timeout);
                    if (!t.Wait(timeout)) {
                        return "(timed out)";
                    }
                }
            } catch (Exception e) {
                EvalOutput.WriteLine(e.ToString());
            }
            return EvalOutput.GetStringBuilder().Length > 0 ? EvalOutput.ToString() : "";
        }
    }

    internal static class StreamHelper {
        public static unsafe int ReadInt32(this Stream s) {
            byte[] bytes = new byte[4];
            s.Read(bytes, 0, 4);
            fixed (byte* intP = bytes)
            {
                return *(int*) intP;
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

        public static unsafe void WriteLengthUTF8(this Stream s, string d) {
            if (d == null) {
                d = "";
            }

            byte[] strBytes = Encoding.UTF8.GetBytes(d);
            byte[] len = new byte[4];

            fixed (byte* lenCP = len)
            {
                int* lenP = (int*) lenCP;
                *lenP = strBytes.Length;
            }

            s.Write(len, 0, 4);
            s.Write(strBytes, 0, strBytes.Length);
        }
    }
}
