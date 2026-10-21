"""Decodes every Tight JPEG rectangle in a recording with Pillow (libjpeg), as an expected value
independent of justvnc's decoder.

Usage: python tight_jpeg_reference.py FIXTURE.server.bin OUT.bin [RAW_REFERENCE.server.bin]

OUT holds, for each JPEG rectangle in order: x, y, width, height as big-endian U16, then
width*height red, green, blue bytes. With a Raw reference recording of the same screen, it also
prints how far the JPEG pixels are from the lossless ones.

The recording is an RFB 3.8 None handshake (one security type) and FramebufferUpdates of Tight,
LastRect and Raw rectangles, with 3-byte TPIXELs.
"""
import io
import struct
import sys

from PIL import Image


def compact_length(b, i):
    value = 0
    for n, shift in enumerate((0, 7, 14)):
        byte = b[i + n]
        value |= (byte if n == 2 else byte & 0x7F) << shift
        if n == 2 or not byte & 0x80:
            return value, i + n + 1


def server_init_end(b):
    i = 12 + 1 + b[12] + 4
    return i, i + 24 + struct.unpack(">I", b[i + 20:i + 24])[0]


def jpeg_rects(b):
    _, i = server_init_end(b)
    while i < len(b):
        rects = struct.unpack(">H", b[i + 2:i + 4])[0]
        i += 4
        n = 0
        while rects == 0xFFFF or n < rects:
            n += 1
            x, y, w, h, enc = struct.unpack(">HHHHi", b[i:i + 12])
            i += 12
            if enc == -224:
                break
            if enc == 0:
                i += w * h * 4
                continue
            assert enc == 7, enc
            control = b[i]
            i += 1
            method = control >> 4
            if method == 0x8:
                i += 3
            elif method == 0x9:
                length, i = compact_length(b, i)
                yield x, y, w, h, b[i:i + length]
                i += length
            else:
                raise SystemExit("this script reads fill and JPEG rectangles only")


def raw_screen(path):
    b = open(path, "rb").read()
    start, i = server_init_end(b)
    width = struct.unpack(">H", b[start:start + 2])[0]
    i += 4
    x, y, w, h, enc = struct.unpack(">HHHHi", b[i:i + 12])
    assert (x, y, enc) == (0, 0, 0)
    return width, b[i + 12:i + 12 + w * h * 4]


def main(fixture, out, raw=None):
    b = open(fixture, "rb").read()
    result = bytearray()
    screen = raw_screen(raw) if raw else None
    for x, y, w, h, jfif in jpeg_rects(b):
        image = Image.open(io.BytesIO(jfif))
        assert image.size == (w, h), (image.size, w, h)
        rgb = image.convert("RGB").tobytes()
        result += struct.pack(">HHHH", x, y, w, h) + rgb
        line = f"JPEG {w}x{h} at ({x}, {y}), mode {image.mode}, {len(jfif)} bytes"
        if screen:
            width, pixels = screen
            diffs = []
            for row in range(h):
                for col in range(w):
                    o = ((y + row) * width + x + col) * 4
                    exact = (pixels[o + 2], pixels[o + 1], pixels[o])
                    got = rgb[(row * w + col) * 3:(row * w + col) * 3 + 3]
                    diffs += [abs(a - b) for a, b in zip(got, exact)]
            line += f"; vs Raw: mean {sum(diffs) / len(diffs):.2f}, max {max(diffs)}"
        print(line)
    open(out, "wb").write(result)


if __name__ == "__main__":
    main(*sys.argv[1:])
