"""Counts the ZRLE subencodings in a recorded server stream, and checks each tile parses exactly.

Usage: python zrle_census.py FIXTURE.server.bin

The stream is an RFB 3.8 None handshake followed by FramebufferUpdates. CPIXELs are assumed to be
3 bytes (32 bpp, depth 24), which is what x11vnc and TigerVNC send in their native format.
"""
import collections
import struct
import sys
import zlib


def main(path):
    b = open(path, "rb").read()
    i = 12 + 1 + b[12] + 4
    width, height = struct.unpack(">HH", b[i:i + 4])
    i += 24 + struct.unpack(">I", b[i + 20:i + 24])[0]
    z, kinds, updates = zlib.decompressobj(), collections.Counter(), 0
    while i < len(b):
        rects = struct.unpack(">H", b[i + 2:i + 4])[0]
        i += 4
        updates += 1
        for _ in range(rects):
            _, _, w, h, enc = struct.unpack(">HHHHi", b[i:i + 12])
            i += 12
            assert enc == 16, enc
            n = struct.unpack(">I", b[i:i + 4])[0]
            data = z.decompress(b[i + 4:i + 4 + n])
            i += 4 + n
            j = 0
            for ty in range(0, h, 64):
                for tx in range(0, w, 64):
                    tw, th = min(64, w - tx), min(64, h - ty)
                    sub = data[j]
                    j += 1

                    def run_length():
                        nonlocal j
                        length = 1
                        while True:
                            byte = data[j]
                            j += 1
                            length += byte
                            if byte != 255:
                                return length

                    def runs(palette):
                        nonlocal j
                        left = tw * th
                        while left > 0:
                            if palette:
                                index = data[j]
                                j += 1
                                left -= run_length() if index & 0x80 else 1
                            else:
                                j += 3
                                left -= run_length()
                        assert left == 0

                    if sub == 0:
                        j += tw * th * 3
                        kinds["raw"] += 1
                    elif sub == 1:
                        j += 3
                        kinds["solid"] += 1
                    elif 2 <= sub <= 16:
                        bits = 1 if sub == 2 else 2 if sub <= 4 else 4
                        j += sub * 3 + (tw * bits + 7) // 8 * th
                        kinds["packed palette"] += 1
                    elif sub == 128:
                        runs(False)
                        kinds["plain RLE"] += 1
                    elif sub >= 130:
                        j += (sub - 128) * 3
                        runs(True)
                        kinds["palette RLE"] += 1
                    else:
                        raise SystemExit(f"subencoding {sub} is not allowed in ZRLE")
            assert j == len(data), (j, len(data))
    print(f"{width}x{height}, {updates} updates: {dict(kinds)}")


if __name__ == "__main__":
    main(sys.argv[1])
