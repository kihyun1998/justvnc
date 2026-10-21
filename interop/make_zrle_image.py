"""Writes a 256x128 XWD image (ZPixmap, depth 24, 32 bpp, LSBFirst) for ZRLE recordings.

Usage: python make_zrle_image.py OUT.xwd

Each 64x64 tile holds a different kind of content (noise, ramps, runs, a few colours, a solid
colour, a checker) so a server's ZRLE encoder has a reason to pick each subencoding. Show it on an
X display with `xwud -in OUT.xwd -geometry +0+0`.
"""
import random, struct, sys
W, H = 256, 128
random.seed(7)
def pixel(x, y):
    tx = x // 64
    if y < 64:
        if tx == 0: return (random.randrange(256), random.randrange(256), random.randrange(256))  # noise
        if tx == 1: return (x * 4 % 256, (y * 4) % 256, (x + y) % 256 if (x // 2) % 2 else 255 - (x + y) % 256)  # many colours, short runs
        if tx == 2: return ((x // 3) * 50 % 256, 80, 200)  # runs of a few pixels, 22 colours
        return ((x ^ y) & 1) * 255, 0, 0  # 2-colour checker
    if tx == 0: return (30, 140, 60)  # solid
    if tx == 1: return ((x * 7 + y * 13) % 256, (x * 3) % 256, 128)  # ramp, many colours
    if tx == 2: return ((x // 8 + y // 8) % 3 * 100, 50, 50)  # 3 colours in blocks
    return (((x // 4) * 37 + y) % 256, ((x // 4) * 11) % 256, 0)  # 200+ colours in 4-pixel runs
name = b"justvnc\0"
header_size = 100 + len(name)
fields = [header_size, 7, 2, 24, W, H, 0, 0, 32, 0, 32, 32, W * 4, 4, 0xff0000, 0xff00, 0xff, 8, 256, 0, W, H, 0, 0, 0]
data = struct.pack(">25I", *fields) + name
for y in range(H):
    for x in range(W):
        r, g, b = pixel(x, y)
        data += struct.pack("<I", (r << 16) | (g << 8) | b)
open(sys.argv[1], "wb").write(data)
