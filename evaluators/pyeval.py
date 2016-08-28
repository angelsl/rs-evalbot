#!/usr/bin/env python3
import io, sys, struct, os, traceback, socketserver, socket
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
            sys.stdout = out
            sys.stderr = out
            sys.stdin = None
            sys.__stdout__ = out
            sys.__stderr__ = out
            sys.__stdin__ = None

            result = etor.runsource(code)
        except:
            traceback.print_exc(file=out)
        finally:
            pipe.send((result, out.getvalue()))

class PyEvalServer(socketserver.UnixStreamServer):
    def server_bind(self):
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)

        try:
            os.unlink(self.server_address)
        except OSError:
            if os.path.exists(self.server_address):
              raise

        socketserver.UnixStreamServer.server_bind(self)
        os.chmod(self.server_address, 0o777)
        return

class RequestHandler(socketserver.StreamRequestHandler):
    def handle(self):
        try:
            self.handle_int()
        finally:
            self.request.shutdown()
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
        if not pipe.poll(timeout / 1000):
            # no result after timeout seconds
            thread.terminate()
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
    server = PyEvalServer(sys.argv[1], RequestHandler)
    server.serve_forever()

if __name__ == "__main__":
    main()
