#!/usr/bin/env python3
import io, sys, struct, os, traceback, socketserver, socket, fcntl, readline

def remove_nl(s):
    if s.endswith("\n"):
        return s[:-1]
    return s

while True:
    sock = socket.socket(family=socket.AF_UNIX)
    sock.connect(sys.argv[1])
    con = b"test-stdin"
    try:
        inp = bytes(input('> '), 'utf-8')
    except EOFError:
        os._exit(0)
    sock.sendall(struct.pack('III', 0, len(con), len(inp)))
    sock.sendall(con)
    sock.sendall(inp)

    outlen, = struct.unpack('I', sock.recv(4))
    print(remove_nl(sock.recv(outlen).decode('utf-8')))
