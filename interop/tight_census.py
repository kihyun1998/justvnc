"""Counts the Tight compression methods and filters in a recorded server stream.

Usage: python tight_census.py FIXTURE.server.bin

The stream is an RFB 3.8 None handshake followed by FramebufferUpdates of Tight (7), Raw (0),
CopyRect (1), DesktopSize (-223) and LastRect (-224) rectangles. Every Tight rectangle is parsed
from its explicit sizes and compact lengths alone, as framing must, then its zlib data is
inflated on the stream it names, honouring reset bits, to check the inflated size matches.
TPIXELs are taken as 3 bytes (true colour, 32 bpp, depth 24, 8-bit components).
"""
import collections
import struct
import sys
import zlib


def compact_length(b, i):
    value, shift = 0, 0
    for n in range(3):
        byte = b[i + n]
        value |= (byte & (0x7F if n < 2 else 0xFF)) << shift
        shift += 7
        if n == 2 or not byte & 0x80:
            return value, i + n + 1


def main(path):
    b = open(path, "rb").read()
    i = 12 + 1 + b[12] + 4
    width, height = struct.unpack(">HH", b[i:i + 4])
    i += 24 + struct.unpack(">I", b[i + 20:i + 24])[0]
    streams = [zlib.decompressobj() for _ in range(4)]
    kinds, other, updates = collections.Counter(), collections.Counter(), 0
    while i < len(b):
        rects = struct.unpack(">H", b[i + 2:i + 4])[0]
        i += 4
        updates += 1
        n = 0
        while rects == 0xFFFF or n < rects:
            n += 1
            _, _, w, h, enc = struct.unpack(">HHHHi", b[i:i + 12])
            i += 12
            if enc == -224:
                other["LastRect"] += 1
                break
            if enc == 0:
                i += w * h * 4
                other["Raw"] += 1
                continue
            if enc == 1:
                i += 4
                other["CopyRect"] += 1
                continue
            if enc == -223:
                other["DesktopSize"] += 1
                continue
            assert enc == 7, enc
            control = b[i]
            i += 1
            for s in range(4):
                if control & (1 << s):
                    streams[s] = zlib.decompressobj()
                    kinds[f"reset stream {s}"] += 1
            method = control >> 4
            if method == 0x8:
                i += 3
                kinds["fill"] += 1
                continue
            if method == 0x9:
                length, i = compact_length(b, i)
                i += length
                kinds["jpeg"] += 1
                continue
            assert method & 0x8 == 0, f"compression-control {control:#x}"
            stream = method & 0x3
            filter_id = 0
            if method & 0x4:
                filter_id = b[i]
                i += 1
            if filter_id == 1:
                colours = b[i] + 1
                i += 1 + colours * 3
                size = (w + 7) // 8 * h if colours == 2 else w * h
                name = f"palette ({'2 colours' if colours == 2 else f'{colours} colours'})"
            else:
                size = w * h * 3
                name = {0: "copy", 2: "gradient"}[filter_id]
            if size < 12:
                i += size
                kinds[f"{name}, uncompressed"] += 1
                continue
            length, i = compact_length(b, i)
            inflated = streams[stream].decompress(b[i:i + length])
            assert len(inflated) == size, (name, len(inflated), size)
            i += length
            kinds[f"{name}, stream {stream}"] += 1
    print(f"{width}x{height}, {updates} updates")
    for k, v in sorted({**kinds, **other}.items()):
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main(sys.argv[1])
