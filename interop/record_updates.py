"""Records framebuffer updates from a real VNC server while X commands run inside its container.

Usage: python record_updates.py HOST PORT OUT_PREFIX --exec "podman exec NAME"
           [--encodings N,N,...] [--pixel-format F] STEP [STEP ...]

STEP is one of:
  full         send a full FramebufferUpdateRequest for the whole screen and read the update
  incremental  the same, incremental
  run:CMD      run CMD in the container with DISPLAY=:0, then wait half a second

The client side is scripted: an RFB 3.8 handshake with security type None, ClientInit with the
shared flag set, and SetEncodings: by default (DesktopSize, LastRect, CopyRect, Tight, ZRLE, Raw), the list justvnc's
client session sends, or --encodings. Raw, CopyRect, ZRLE, Tight, DesktopSize and LastRect
rectangles are read, ZRLE and Tight only by their sizes and lengths (Tight with 3-byte TPIXELs);
any other encoding stops the script. --pixel-format sends SetPixelFormat before SetEncodings;
F is bits-per-pixel,depth,big-endian,red-max,green-max,blue-max,red-shift,green-shift,blue-shift
(true colour), e.g. 16,16,0,31,63,31,11,5,0.
Server bytes go to OUT_PREFIX.server.bin and client bytes to OUT_PREFIX.client.bin.
"""
import argparse
import shlex
import socket
import struct
import subprocess
import time


def read_compact_length(read):
    value = 0
    for n, shift in enumerate((0, 7, 14)):
        byte = read(1)[0]
        value |= (byte if n == 2 else byte & 0x7F) << shift
        if n == 2 or not byte & 0x80:
            return value


def read_tight(read, w, h, tpixel):
    """Reads one Tight rectangle's data from its sizes alone (rfbproto, Tight Encoding)."""
    control = read(1)[0]
    method = control >> 4
    if method == 0x8:
        read(tpixel)
        return
    if method == 0x9:
        read(read_compact_length(read))
        return
    if method & 0x8:
        raise SystemExit(f"unexpected Tight compression-control {control:#x}")
    filter_id = read(1)[0] if method & 0x4 else 0
    if filter_id == 1:
        colours = read(1)[0] + 1
        read(colours * tpixel)
        size = (w + 7) // 8 * h if colours == 2 else w * h
    else:
        size = w * h * tpixel
    read(size if size < 12 else read_compact_length(read))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("host")
    p.add_argument("port", type=int)
    p.add_argument("out_prefix")
    p.add_argument("--exec", required=True, dest="exec_prefix")
    p.add_argument("--encodings", default="-223,-224,1,7,16,0")
    p.add_argument("--pixel-format")
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
    if a.pixel_format:
        bpp, depth, be, rmax, gmax, bmax, rs, gs, bs = (int(v) for v in a.pixel_format.split(","))
        send(struct.pack(">B3xBBBBHHHBBB3x", 0, bpp, depth, be, 1, rmax, gmax, bmax, rs, gs, bs))
        bytes_per_pixel = bpp // 8
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
        # 0xFFFF rectangles: as many as come before a LastRect (-224).
        while rects == 0xFFFF or len(kinds) < rects:
            _, _, w, h, encoding = struct.unpack(">HHHHi", read(12))
            kinds.append(encoding)
            if encoding == 0:
                read(w * h * bytes_per_pixel)
            elif encoding == 1:
                read(4)
            elif encoding == 16:
                read(int.from_bytes(read(4), "big"))
            elif encoding == 7:
                read_tight(read, w, h, 3)
            elif encoding == -223:
                width, height = w, h
            elif encoding == -224:
                break
            else:
                raise SystemExit(f"unexpected encoding {encoding}")
        print(f"{step}: {rects} rectangles, encodings {kinds}")

    sock.close()
    for suffix, data in (("server", server), ("client", client)):
        with open(f"{a.out_prefix}.{suffix}.bin", "wb") as f:
            f.write(data)
    print(f"server {len(server)} bytes, client {len(client)} bytes -> {a.out_prefix}.*.bin")


if __name__ == "__main__":
    main()
