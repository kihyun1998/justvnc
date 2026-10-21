"""Writes the small XWD images the Tight recording shows next to make_zrle_image.py's image.

Usage: python make_tight_images.py OUT_DIR

  tight-mono.xwd     64x64, two colours   (a server's Tight palette filter, 1 bit per pixel)
  tight-indexed.xwd  64x64, twenty colours (palette filter, 1 byte per pixel)
  tight-noise.xwd    64x64, random colours (copy filter: nothing to predict, nothing to index)
"""
import os
import random
import struct
import sys


def write_xwd(path, width, height, pixel):
    """ZPixmap, depth 24, 32 bpp, LSBFirst; pixel(x, y) returns (r, g, b)."""
    name = b"justvnc\0"
    fields = [100 + len(name), 7, 2, 24, width, height, 0, 0, 32, 0, 32, 32, width * 4, 4,
              0xff0000, 0xff00, 0xff, 8, 256, 0, width, height, 0, 0, 0]
    data = bytearray(struct.pack(">25I", *fields) + name)
    for y in range(height):
        for x in range(width):
            r, g, b = pixel(x, y)
            data += struct.pack("<I", (r << 16) | (g << 8) | b)
    with open(path, "wb") as f:
        f.write(data)


def main(out_dir):
    random.seed(11)
    write_xwd(os.path.join(out_dir, "tight-mono.xwd"), 64, 64,
              lambda x, y: (250, 250, 250) if (x // 3 + y // 5) % 2 else (10, 20, 90))
    write_xwd(os.path.join(out_dir, "tight-indexed.xwd"), 64, 64,
              lambda x, y: ((x // 4 + y // 16 * 4) % 20 * 12, 100, 200 - (x // 4) % 20 * 9))
    write_xwd(os.path.join(out_dir, "tight-noise.xwd"), 64, 64,
              lambda x, y: (random.randrange(256), random.randrange(256), random.randrange(256)))


if __name__ == "__main__":
    main(sys.argv[1])
