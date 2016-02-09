#!/usr/bin/env python3
import io, sys, struct, os, traceback
from multiprocessing import Process, Pipe
from code import InteractiveInterpreter

class PyEval(InteractiveInterpreter):
    def __init__(self, locals=None):
        InteractiveInterpreter.__init__(self, locals)

class EmptyIO(io.RawIOBase):
    def __init__(self):
        io.RawIOBase(self)

    def readinto(x):
        return 0

    def write(x):
        return 0

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
    out = opt.encode('utf-8')
    outlen = struct.pack('I', len(out))
    outbuf.write(outlen)
    outbuf.write(out)
    outbuf.flush()

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

def main():
    inbuf = os.fdopen(0, mode='rb')
    outbuf = os.fdopen(1, mode='wb')
    stderr = os.fdopen(2, mode='wb')

    # make it harder to hijack stdin/out
    dummy = EmptyIO()
    sys.stdout = dummy
    sys.__stdout__ = dummy
    sys.stderr = dummy
    sys.__stderr__ = dummy
    sys.stdin = dummy
    sys.__stdin__ = dummy

    while True:
        codebufs = {}
        pipe, childpipe = Pipe()
        thread = Process(target=worker, args=(childpipe,), daemon=True)
        thread.start()
        while True:
            timeout, key, inp = readinput(inbuf)
            codebuf = codebufs.setdefault(key, [])
            codebuf.append(inp)
            source = '\n'.join(codebuf)

            pipe.send((source, key))
            if not pipe.poll(timeout / 1000):
                # there is no result after timeout seconds
                thread.terminate()
                writeoutput(outbuf, "(timed out)")
                break

            try:
                # poll = True may mean the pipe is closed
                more, result = pipe.recv()
            except EOFError:
                # yup, the pipe was closed.
                writeoutput(outbuf, "(worker process died)")
                break
            except:
                writeoutput(outbuf, "(exception @ python main)")
                traceback.print_exc(file=stderr)
                break

            if not more:
                codebuf.clear()
                writeoutput(outbuf, result)
            elif more:
                writeoutput(outbuf, "(continue...)")
            else:
                writeoutput(outbuf, "something weird happened")

if __name__ == "__main__":
    main()
