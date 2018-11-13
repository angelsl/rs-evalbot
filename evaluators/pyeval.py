#!/usr/bin/env python3
import io, sys, struct, os, traceback, socketserver, socket, fcntl
import contextlib, multiprocessing
from multiprocessing import Process, Pipe
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


def worker(pipe):
    etors = {}
    while True:
        code, key = pipe.recv()
        etor = etors.setdefault(key, PyEval())
        try:
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                with contextlib.redirect_stderr(out):
                    result = etor.runsource(code)
        except:
            traceback.print_exc(file=out)
        finally:
            pipe.send((result, out.getvalue()))

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
        global pipe, childpipe, codebufs, thread

        if thread is None or not thread.is_alive():
            thread = Process(target=worker, args=(childpipe,), daemon=True)
            thread.start()

        timeout, key, codefragment = readinput(self.rfile)
        codebuf = codebufs.setdefault(key, [])
        codebuf.append(codefragment)
        source = '\n'.join(codebuf)

        pipe.send((source, key))
        if timeout > 0 and not pipe.poll(timeout / 1000):
            # no result after timeout seconds
            thread.kill()
            thread.join()
            thread.close()
            thread = None
            codebuf.clear()
            writeoutput(self.wfile, "(timed out)")
            return

        try:
            # pipe may be closed even though poll() returns True
            more, result = pipe.recv()
        except EOFError:
            codebuf.clear()
            writeoutput(self.wfile, "(worker process died)")
            return
        except:
            codebuf.clear()
            writeoutput(self.wfile, "(unexpected exception)")
            traceback.print_exc(file=sys.stderr)
            return

        if not more:
            codebuf.clear()
            writeoutput(self.wfile, result)
        elif more:
            writeoutput(self.wfile, "(continue...)")
        else:
            codebuf.clear()
            writeoutput(self.wfile, "something weird happened")

codebufs = {}
pipe, childpipe = Pipe()
thread = None

def main():
    multiprocessing.set_start_method('spawn')
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
