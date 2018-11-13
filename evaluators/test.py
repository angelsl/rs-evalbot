#!/usr/bin/env python3
import io, sys, struct, os, traceback, socketserver, socket, fcntl

sock = socket.socket(family=socket.AF_UNIX)
sock.connect(sys.argv[1])
con = b"test-stdin"
inp = bytes(sys.stdin.read(), 'utf-8')
sock.sendall(struct.pack('III', 0, len(con), len(inp)))
sock.sendall(con)
sock.sendall(inp)
sock.flush()

outlen, = struct.unpack('I', sock.recv(4))
print(sock.recv(outlen))
