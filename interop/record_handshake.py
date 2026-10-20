"""Records the bytes a VNC server sends during an RFB 3.8 handshake with security type None.

Usage: python record_handshake.py HOST PORT OUT_FILE

The client side is scripted: it answers "RFB 003.008\n", picks security type 1 and sends
ClientInit with the shared flag set. Every byte the server sends until ServerInit is complete,
or until the server closes the connection, is written to OUT_FILE.
"""
import socket
import sys


def main(host, port, out):
    sock = socket.create_connection((host, int(port)), timeout=10)
    received = bytearray()

    def read(n):
        data = b""
        while len(data) < n:
            chunk = sock.recv(n - len(data))
            if not chunk:
                raise EOFError
            data += chunk
        received.extend(data)
        return data

    try:
        read(12)
        sock.sendall(b"RFB 003.008\n")
        count = read(1)[0]
        if count == 0:
            read(int.from_bytes(read(4), "big"))
            return
        read(count)
        sock.sendall(bytes([1]))
        if int.from_bytes(read(4), "big") != 0:
            read(int.from_bytes(read(4), "big"))
            return
        sock.sendall(bytes([1]))
        header = read(24)
        read(int.from_bytes(header[20:24], "big"))
    except EOFError:
        pass
    finally:
        sock.close()
        with open(out, "wb") as f:
            f.write(received)
        print(f"{len(received)} bytes -> {out}")


if __name__ == "__main__":
    main(*sys.argv[1:])
