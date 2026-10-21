"""Records framebuffer updates from a real VNC server while X commands run inside its container.

Usage: python record_updates.py HOST PORT OUT_PREFIX --exec "podman exec NAME"
           [--encodings N,N,...] STEP [STEP ...]

STEP is one of:
  full         send a full FramebufferUpdateRequest for the whole screen and read the update
  incremental  the same, incremental
  run:CMD      run CMD in the container with DISPLAY=:0, then wait half a second

The client side is scripted: an RFB 3.8 handshake with security type None, ClientInit with the
shared flag set, and SetEncodings: by default (DesktopSize, CopyRect, ZRLE, Raw), the list justvnc's
client session sends, or --encodings. Raw, CopyRect, ZRLE and DesktopSize rectangles are read (ZRLE
only by its length prefix); any other encoding stops the script.
Server bytes go to OUT_PREFIX.server.bin and client bytes to OUT_PREFIX.client.bin.
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
    p.add_argument("--encodings", default="-223,1,16,0")
    p.add_argument("steps", nargs="+")
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
    width, height = struct.unpack(">HH", header[:4])
    bytes_per_pixel = header[4] // 8
    encodings = [int(e) for e in a.encodings.split(",")]
    send(struct.pack(f">BBH{len(encodings)}i", 2, 0, len(encodings), *encodings))

    for step in a.steps:
        if step.startswith("run:"):
            command = step[len("run:"):]
            subprocess.run(
                shlex.split(a.exec_prefix) + ["sh", "-c", f"export DISPLAY=:0; {command}"],
                check=True,
            )
            time.sleep(0.5)
            continue
        incremental = {"full": 0, "incremental": 1}[step]
        send(struct.pack(">BBHHHH", 3, incremental, 0, 0, width, height))
        assert read(1) == b"\0", "expected a FramebufferUpdate"
        _, rects = struct.unpack(">BH", read(3))
        kinds = []
        for _ in range(rects):
            _, _, w, h, encoding = struct.unpack(">HHHHi", read(12))
            if encoding == 0:
                read(w * h * bytes_per_pixel)
            elif encoding == 1:
                read(4)
            elif encoding == 16:
                read(int.from_bytes(read(4), "big"))
            elif encoding == -223:
                width, height = w, h
            else:
                raise SystemExit(f"unexpected encoding {encoding}")
            kinds.append(encoding)
        print(f"{step}: {rects} rectangles, encodings {kinds}")

    sock.close()
    for suffix, data in (("server", server), ("client", client)):
        with open(f"{a.out_prefix}.{suffix}.bin", "wb") as f:
            f.write(data)
    print(f"server {len(server)} bytes, client {len(client)} bytes -> {a.out_prefix}.*.bin")


if __name__ == "__main__":
    main()
