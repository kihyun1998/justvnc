"""Records what a VNC server sends between updates while X commands run inside its container.

Usage: python record_server_messages.py HOST PORT OUT_PREFIX --exec "podman exec NAME" CMD [CMD ...]

The client side is scripted: an RFB 3.8 handshake with security type None, ClientInit with the
shared flag set, then SetEncodings (Raw) and nothing else, so the server sends no updates. Each
CMD then runs in the container with DISPLAY=:0, and whatever the server sends within a second of
it is kept. Server bytes go to OUT_PREFIX.server.bin and client bytes to OUT_PREFIX.client.bin.
"""
import argparse
import shlex
import socket
import struct
import subprocess
import time


def main():
    p = argparse.ArgumentParser()
    p.add_argument("host")
    p.add_argument("port", type=int)
    p.add_argument("out_prefix")
    p.add_argument("--exec", required=True, dest="exec_prefix")
    p.add_argument("commands", nargs="+")
    a = p.parse_args()

    sock = socket.create_connection((a.host, a.port), timeout=10)
    server, client = bytearray(), bytearray()

    def read(n):
        data = b""
        while len(data) < n:
            chunk = sock.recv(n - len(data))
            if not chunk:
                raise EOFError
            data += chunk
        server.extend(data)
        return data

    def send(data):
        client.extend(data)
        sock.sendall(data)

    read(12)
    send(b"RFB 003.008\n")
    read(read(1)[0])
    send(b"\x01")
    assert read(4) == b"\0\0\0\0", "security failed"
    send(b"\x01")
    header = read(24)
    read(int.from_bytes(header[20:24], "big"))
    send(struct.pack(">BBHi", 2, 0, 1, 0))

    sock.settimeout(1.0)
    for command in a.commands:
        subprocess.run(shlex.split(a.exec_prefix) + ["sh", "-c", f"export DISPLAY=:0; {command}"], check=True)
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            try:
                chunk = sock.recv(4096)
            except socket.timeout:
                break
            if not chunk:
                break
            server.extend(chunk)
    sock.close()
    for suffix, data in (("server", server), ("client", client)):
        with open(f"{a.out_prefix}.{suffix}.bin", "wb") as f:
            f.write(data)
    print(f"server {len(server)} bytes, client {len(client)} bytes -> {a.out_prefix}.*.bin")


if __name__ == "__main__":
    main()
