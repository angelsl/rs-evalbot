using System;
using System.IO;
using System.Text;
using System.Net;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;
using System.Runtime.InteropServices;
using System.Diagnostics;
using System.Reflection;
using System.Collections.Generic;
using Microsoft.CodeAnalysis.Scripting;
using Microsoft.CodeAnalysis.Scripting.Hosting;
using Microsoft.CodeAnalysis.CSharp.Scripting;
using Microsoft.CodeAnalysis.CSharp.Scripting.Hosting;

namespace cseval {
    static class Extensions {
        // Adapted from https://stackoverflow.com/a/27852439
        public static async void FireAndForget(this Task task){
            try {
                await task;
            } catch (Exception e) {
                Console.Error.WriteLine("Exception: {0}", e);
            }
        }
    }

    [StructLayout(LayoutKind.Explicit, Pack=4)]
    struct RequestHeader {
        public const int Size = 12;

        [FieldOffset(0)] public uint Timeout;
        [FieldOffset(4)] public uint ContextKeyLength;
        [FieldOffset(8)] public uint CodeLength;

        public static unsafe RequestHeader FromBytes(byte[] arr) {
            RequestHeader r;
            fixed (byte* arrp = arr) {
                RequestHeader* p = (RequestHeader*) arrp;
                r.Timeout = p->Timeout;
                r.ContextKeyLength = p->ContextKeyLength;
                r.CodeLength = p->CodeLength;
            }
            return r;
        }
    }

    class Program {
        private static readonly ScriptOptions SCRIPT_OPTIONS = ScriptOptions.Default
            .WithEmitDebugInformation(false)
            .WithFilePath("-")
            .AddReferences("System.Linq", "System.Collections")
            .AddImports("System", "System.Linq", "System.Collections.Generic");

        private static readonly PrintOptions PRINT_OPTIONS = new PrintOptions() {
            MaximumOutputLength = 512
        };

        private static readonly Dictionary<string, ScriptState<object>> _contexts =
            new Dictionary<string, ScriptState<object>>();

        async static Task Main(string[] args) {
            Debug.Assert(Marshal.SizeOf(new RequestHeader()) == RequestHeader.Size);
            await AcceptForever(new Socket(new SafeSocketHandle(new IntPtr(3), true)));
        }

        async static Task AcceptForever(Socket socket) {
            while (true) {
                Socket conn = await Task<Socket>.Factory.FromAsync(
                    socket.BeginAccept, socket.EndAccept, null).ConfigureAwait(false);
                HandleConnection(conn).FireAndForget();
            }
        }

        async static Task<bool> ReadExact(Stream s, byte[] buf) {
            int read = 0, read_now = 0;
            while ((read_now = await s.ReadAsync(buf, read, buf.Length - read, CancellationToken.None).ConfigureAwait(false)) != 0) {
                read += read_now;
                if (read >= buf.Length) {
                    break;
                }
            }
            if (read < buf.Length) {
                return false;
            }
            return true;
        }

        async static Task HandleConnection(Socket conn) {
            try {
                NetworkStream ns = new NetworkStream(conn, true);
                byte[] buf = new byte[RequestHeader.Size];
                if (!await ReadExact(ns, buf)) {
                    Console.Error.WriteLine("early eof while reading header");
                    return;
                }
                RequestHeader h = RequestHeader.FromBytes(buf);
                buf = new byte[h.ContextKeyLength + h.CodeLength];
                if (!await ReadExact(ns, buf)) {
                    Console.Error.WriteLine("early eof while reading body");
                    return;
                }
                string conkey = Encoding.UTF8.GetString(buf, 0, (int) h.ContextKeyLength);
                string code = Encoding.UTF8.GetString(buf, (int) h.ContextKeyLength, (int) h.CodeLength);

                // TODO
                string resp = "unknown error";
                ScriptState<object> res = _contexts.GetValueOrDefault(conkey, null);
                try {
                    Task<ScriptState<Object>> tres;
                    if (res != null) {
                        tres = res.ContinueWithAsync(code, SCRIPT_OPTIONS);
                    } else {
                        tres = CSharpScript.RunAsync(code, SCRIPT_OPTIONS);
                    }
                    res = await tres;
                    if (res == null) {
                        resp = "null result?";
                    } else if (res.Exception != null) {
                        resp = CSharpObjectFormatter.Instance.FormatException(res.Exception);
                    } else if (res.ReturnValue != null) {
                        resp = CSharpObjectFormatter.Instance.FormatObject(res.ReturnValue, PRINT_OPTIONS);
                    } else {
                        resp = "";
                    }
                    _contexts[conkey] = res;
                } catch (Exception e) {
                    resp = CSharpObjectFormatter.Instance.FormatException(e);
                }
                byte[] respbytes = Encoding.UTF8.GetBytes(resp);
                await ns.WriteAsync(IntToBytes(respbytes.Length), 0, 4, CancellationToken.None).ConfigureAwait(false);
                await ns.WriteAsync(respbytes, 0, respbytes.Length, CancellationToken.None).ConfigureAwait(false);
            } catch (Exception e) {
                Console.Error.WriteLine("Exception: {0}", e);
            }
        }

        static unsafe byte[] IntToBytes(int n) {
            byte[] r = new byte[sizeof(int)];
            fixed (byte* rp = r) {
                *((int*) rp) = n;
            }
            return r;
        }
    }
}
