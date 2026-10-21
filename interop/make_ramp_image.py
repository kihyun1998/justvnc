"""Writes a 256x16 XWD image (ZPixmap, depth 24, 32 bpp, LSBFirst) of colour ramps, for recording
how a server converts pixel formats.

Usage: python make_ramp_image.py OUT.xwd

Column x holds the value x: rows 0-3 in red, 4-7 in green, 8-11 in blue, 12-15 in all three.
"""
import struct, sys
W, H = 256, 16
def pixel(x, y):
    return [(x, 0, 0), (0, x, 0), (0, 0, x), (x, x, x)][y // 4]
name = b"justvnc\0"
header_size = 100 + len(name)
fields = [header_size, 7, 2, 24, W, H, 0, 0, 32, 0, 32, 32, W * 4, 4, 0xff0000, 0xff00, 0xff, 8, 256, 0, W, H, 0, 0, 0]
data = struct.pack(">25I", *fields) + name
for y in range(H):
    for x in range(W):
        r, g, b = pixel(x, y)
        data += struct.pack("<I", (r << 16) | (g << 8) | b)
open(sys.argv[1], "wb").write(data)
