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
        async static Task Main(string[] args) {
            Debug.Assert(Marshal.SizeOf(new RequestHeader()) == RequestHeader.Size);
            await AcceptForever(CreateSocketFromFd(3));
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
                string resp = string.Format("Context: \"{0}\"\nCode: \"{1}\"\n", conkey, code);
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

        // Following methods adapted from
        // https://github.com/tmds/Tmds.Systemd/blob/master/src/Tmds.Systemd/ServiceManager.Socket.cs

        /*
            ﻿Copyright 2017 Tom Deseyn <tom.deseyn@gmail.com>

            Permission is hereby granted, free of charge, to any person obtaining
            a copy of this software and associated documentation files (the
            "Software"), to deal in the Software without restriction, including
            without limitation the rights to use, copy, modify, merge, publish,
            distribute, sublicense, and/or sell copies of the Software, and to
            permit persons to whom the Software is furnished to do so, subject to
            the following conditions:

            The above copyright notice and this permission notice shall be
            included in all copies or substantial portions of the Software.

            THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
            EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
            MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
            IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
            CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
            TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
            SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
        */

        static ReflectionMethods reflectionMethods;

        static Program() {
            reflectionMethods = LookupMethods();
        }

        static Socket CreateSocketFromFd(int fd) {
            // static unsafe SafeCloseSocket CreateSocket(IntPtr fileDescriptor)
            var fileDescriptor = new IntPtr(fd);
            var safeCloseSocket = reflectionMethods.SafeCloseSocketCreate.Invoke(null, new object [] { fileDescriptor });

            // private Socket(SafeCloseSocket fd)
            var socket = reflectionMethods.SocketConstructor.Invoke(new[] { safeCloseSocket });

            // private bool _isListening = false;
            reflectionMethods.IsListening.SetValue(socket, true);

            // internal EndPoint _rightEndPoint;
            reflectionMethods.RightEndPoint.SetValue(socket,
                (EndPoint) reflectionMethods.UnixDomainSocketEndPointConstructor.Invoke(new[] { "/" }));
            // private AddressFamily _addressFamily;
            reflectionMethods.AddressFamily.SetValue(socket, AddressFamily.Unix);
            // private SocketType _socketType;
            reflectionMethods.SocketType.SetValue(socket, SocketType.Stream);
            // private ProtocolType _protocolType;
            reflectionMethods.ProtocolType.SetValue(socket, ProtocolType.Unspecified);

            return (Socket)socket;
        }

        private class ReflectionMethods {
            public MethodInfo SafeCloseSocketCreate;
            public ConstructorInfo SocketConstructor;
            public FieldInfo RightEndPoint;
            public FieldInfo IsListening;
            public FieldInfo SocketType;
            public FieldInfo AddressFamily;
            public FieldInfo ProtocolType;
            public ConstructorInfo UnixDomainSocketEndPointConstructor;
        }

        private static ReflectionMethods LookupMethods() {
            Assembly socketAssembly = typeof(Socket).GetTypeInfo().Assembly;
            Type safeCloseSocketType = socketAssembly.GetType("System.Net.Sockets.SafeCloseSocket");
            if (safeCloseSocketType == null) {
                ThrowNotSupported(nameof(safeCloseSocketType));
            }
            MethodInfo safeCloseSocketCreate = safeCloseSocketType.GetTypeInfo().GetMethod("CreateSocket", BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public, null, new[] { typeof(IntPtr) }, null);
            if (safeCloseSocketCreate == null) {
                ThrowNotSupported(nameof(safeCloseSocketCreate));
            }
            ConstructorInfo socketConstructor = typeof(Socket).GetTypeInfo().GetConstructor(BindingFlags.Public | BindingFlags.NonPublic| BindingFlags.Instance, null, new[] { safeCloseSocketType }, null);
            if (socketConstructor == null) {
                ThrowNotSupported(nameof(socketConstructor));
            }
            FieldInfo rightEndPoint = typeof(Socket).GetTypeInfo().GetField("_rightEndPoint", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            if (rightEndPoint == null) {
                ThrowNotSupported(nameof(rightEndPoint));
            }
            FieldInfo isListening = typeof(Socket).GetTypeInfo().GetField("_isListening", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            if (isListening == null) {
                ThrowNotSupported(nameof(isListening));
            }
            FieldInfo socketType = typeof(Socket).GetTypeInfo().GetField("_socketType", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            if (socketType == null) {
                ThrowNotSupported(nameof(socketType));
            }
            FieldInfo addressFamily = typeof(Socket).GetTypeInfo().GetField("_addressFamily", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            if (addressFamily == null) {
                ThrowNotSupported(nameof(addressFamily));
            }
            FieldInfo protocolType = typeof(Socket).GetTypeInfo().GetField("_protocolType", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            if (protocolType == null) {
                ThrowNotSupported(nameof(protocolType));
            }

            // .NET Core 2.1+
            Type unixDomainSocketEndPointType = socketAssembly.GetType("System.Net.Sockets.UnixDomainSocketEndPoint");
            if (unixDomainSocketEndPointType == null) {
                ThrowNotSupported(nameof(unixDomainSocketEndPointType));
            }
            ConstructorInfo unixDomainSocketEndPointConstructor = unixDomainSocketEndPointType.GetTypeInfo().GetConstructor(BindingFlags.Public | BindingFlags.NonPublic| BindingFlags.Instance, null, new[] { typeof(string) }, null);
            if (unixDomainSocketEndPointConstructor == null) {
                ThrowNotSupported(nameof(unixDomainSocketEndPointConstructor));
            }
            return new ReflectionMethods {
                SafeCloseSocketCreate = safeCloseSocketCreate,
                SocketConstructor = socketConstructor,
                RightEndPoint = rightEndPoint,
                IsListening = isListening,
                SocketType = socketType,
                AddressFamily = addressFamily,
                ProtocolType = protocolType,
                UnixDomainSocketEndPointConstructor = unixDomainSocketEndPointConstructor
            };
        }

        private static void ThrowNotSupported(string var) {
            throw new NotSupportedException($"Creating a Socket from a file descriptor is not supported on this platform. '{var}' not found.");
        }
    }
}
