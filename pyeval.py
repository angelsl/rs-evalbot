#!/usr/bin/env python3
import io, sys, struct, os, signal
from threading import Thread
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
    timeout, size = struct.unpack('II', inbuf.read(8))
    return (timeout, inbuf.read(size).decode('utf-8'))

def writeoutput(outbuf, success, opt):
    outbuf.write(b'\x01' if success else b'\x00')
    out = opt.encode('utf-8')
    outlen = struct.pack('I', len(out))
    outbuf.write(outlen)
    outbuf.write(out)
    outbuf.flush()

def main():
    etor = PyEval()
    inbuf = os.fdopen(0, mode='rb')
    outbuf = os.fdopen(1, mode='wb')
    
    # make it harder to hijack stdin/out
    dummy = EmptyIO()
    sys.stdout = dummy
    sys.__stdout__ = dummy
    sys.stderr = dummy
    sys.__stderr__ = dummy
    sys.stdin = dummy
    sys.__stdin__ = dummy

    codebuf = []
    while True:
        timeout, inp = readinput(inbuf)
        codebuf.append(inp)
        source = '\n'.join(codebuf)

        def work(req):
            out = io.StringIO()
            sys.stdout = out
            sys.stderr = out
            sys.stdin = None
            sys.__stdout__ = out
            sys.__stderr__ = out
            sys.__stdin__ = None

            result = req.etor.runsource(req.source)
            req.result = result
            req.output = out.getvalue()

        request = Request(etor, source)
        thread = Thread(target=work, args=(request,))
        thread.start()
        thread.join(timeout)
        if thread.is_alive():
            signal.pthread_kill(thread.ident, signal.SIGTERM)
            codebuf = []
            writeoutput(outbuf, False, "(timed out)")
            continue

        more = request.result
        result = request.output
        if not more:
            codebuf = []
            writeoutput(outbuf, True, result)
        elif more:
            writeoutput(outbuf, False, "(continue...)")
        else:
            writeoutput(outbuf, False, "something weird happened")

if __name__ == "__main__":
    main()
