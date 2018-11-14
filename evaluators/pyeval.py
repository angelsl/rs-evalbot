#!/usr/bin/env python3
import io, sys, struct, os, traceback, socketserver, socket, fcntl
import contextlib
from code import InteractiveInterpreter

class PyEval(InteractiveInterpreter):
    def __init__(self, locals=None):
        InteractiveInterpreter.__init__(self, locals)

class Request:
    def __init__(self, etor, source):
        self.etor = etor
        self.source = source
        self.result = None
        self.output = None

def readinput(inbuf):
    timeout, keysize, codesize = struct.unpack('III', inbuf.read(12))
    key = inbuf.read(keysize).decode('utf-8')
    code = inbuf.read(codesize).decode('utf-8')
    return (timeout, key, code)

def writeoutput(outbuf, opt):
    try:
        out = opt.encode('utf-8')
        outlen = struct.pack('I', len(out))
        outbuf.write(outlen)
        outbuf.write(out)
        outbuf.flush()
    except:
        print("error returning output:")
        traceback.print_exc(file=sys.stderr)

class PyEvalServer(socketserver.UnixStreamServer):
    def server_bind(self):
        os.set_inheritable(3, False)
        self.socket = socket.socket(fileno=3)
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)

    def socket_close(self):
        pass # don't close systemd socket!

class RequestHandler(socketserver.StreamRequestHandler):
    def handle(self):
        try:
            self.handle_int()
        finally:
            self.request.shutdown(socket.SHUT_RDWR)
            self.request.close()

    def handle_int(self):
        global codebufs, etors

        timeout, key, codefragment = readinput(self.rfile)
        codebuf = codebufs.setdefault(key, [])
        etor = etors.setdefault(key, PyEval())

        codebuf.append(codefragment)
        source = '\n'.join(codebuf)

        try:
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                with contextlib.redirect_stderr(out):
                    more = etor.runsource(source)
        except:
            traceback.print_exc(file=out)

        if not more:
            codebuf.clear()
            writeoutput(self.wfile, out.getvalue())
        elif more:
            writeoutput(self.wfile, "(continue...)")
        else:
            codebuf.clear()
            writeoutput(self.wfile, "something weird happened")

etors = {}
codebufs = {}

def main():
    server = PyEvalServer(None, RequestHandler, False)
    try:
        server.server_bind()
        server.server_activate()
    except:
        server.server_close()
        raise
    server.serve_forever()

if __name__ == "__main__":
    main()
