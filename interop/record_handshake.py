"""Records both directions of an RFB handshake with a real server, up to ServerInit.

Usage: python record_handshake.py HOST PORT OUT_PREFIX [--version 3.3|3.7|3.8] [--password PW]

The client side is scripted: it answers with --version (default 3.8), picks the first security
type the server offers that it can do (None, or VNC authentication when --password is given),
answers VNC authentication with pycryptodome's DES, and sends ClientInit with the shared flag
set. Server bytes go to OUT_PREFIX.server.bin and client bytes to OUT_PREFIX.client.bin. It stops
at the end of ServerInit or when the server closes.
"""
import argparse
import socket

from Crypto.Cipher import DES


def vnc_auth_response(password, challenge):
    key = password.encode("latin-1")[:8].ljust(8, b"\0")
    key = bytes(int(f"{b:08b}"[::-1], 2) for b in key)
    return DES.new(key, DES.MODE_ECB).encrypt(challenge)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("host")
    p.add_argument("port", type=int)
    p.add_argument("out_prefix")
    p.add_argument("--version", default="3.8", choices=["3.3", "3.7", "3.8"])
    p.add_argument("--password")
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

    def u32():
        return int.from_bytes(read(4), "big")

    def send(data):
        client.extend(data)
        sock.sendall(data)

    try:
        read(12)
        send(f"RFB 003.00{a.version[-1]}\n".encode())
        if a.version == "3.3":
            chosen = u32()
            if chosen == 0:
                read(u32())
                return
        else:
            count = read(1)[0]
            if count == 0:
                read(u32())
                return
            offered = read(count)
            usable = [t for t in offered if t == 1 or (t == 2 and a.password is not None)]
            if not usable:
                return
            chosen = usable[0]
            send(bytes([chosen]))
        if chosen == 2:
            send(vnc_auth_response(a.password, read(16)))
        if chosen == 2 or a.version == "3.8":
            if u32() != 0:
                if a.version == "3.8":
                    read(u32())
                return
        send(bytes([1]))
        header = read(24)
        read(int.from_bytes(header[20:24], "big"))
    except EOFError:
        pass
    finally:
        sock.close()
        for suffix, data in (("server", server), ("client", client)):
            with open(f"{a.out_prefix}.{suffix}.bin", "wb") as f:
                f.write(data)
        print(f"server {len(server)} bytes, client {len(client)} bytes -> {a.out_prefix}.*.bin")


if __name__ == "__main__":
    main()
