"""Writes starting corpora for the fuzz targets from the recorded fixtures: what servers sent
the client role, and what the recorder sent them.

Usage: python seed_corpus.py   (from fuzz/; writes corpus/<target>/)
"""
import pathlib
import struct

FIXTURES = pathlib.Path(__file__).parent.parent / "crates/justvnc-client/tests/fixtures"
CORPUS = pathlib.Path(__file__).parent / "corpus"


def write(target, name, data):
    path = CORPUS / target
    path.mkdir(parents=True, exist_ok=True)
    (path / name).write_bytes(data)


def main():
    for fixture in sorted(FIXTURES.glob("*.server.bin")):
        b = fixture.read_bytes()
        name = fixture.name.removesuffix(".server.bin")
        password = 1 if "vncauth" in name else 0
        for chunk in (0, 7):
            write("client_handshake", f"{name}-{chunk}", bytes([password, chunk]) + b)
            write("client_session", f"{name}-{chunk}", bytes([password, chunk]) + b)
        # The recordings with messages are RFB 3.8 with one security type, None.
        if b[:12] != b"RFB 003.008\n" or b[12:14] != b"\x01\x01" or len(b) < 17 + 24:
            continue
        init = 17
        end = init + 24 + struct.unpack(">I", b[init + 20:init + 24])[0]
        if end >= len(b):
            continue
        for last_rect in (0, 1):
            write("server_messages", f"{name}-{last_rect}", b[init + 4:init + 20] + b[init:init + 4]
                  + bytes([last_rect]) + b[end:])

    for fixture in sorted(FIXTURES.glob("*.client.bin")):
        b = fixture.read_bytes()
        name = fixture.name.removesuffix(".client.bin")
        version = {b"RFB 003.003\n": 0, b"RFB 003.007\n": 1}.get(b[:12], 2)
        flags = version | (4 if "vncauth" in name else 0) | (0 if "wrong" in name else 8)
        for chunk in (0, 7):
            write("server_session", f"{name}-{chunk}", bytes([flags, chunk]) + b)
            write("server_session", f"{name}-{chunk}-damage", bytes([flags | 16, chunk]) + b)
            write("server_session", f"{name}-{chunk}-moves", bytes([flags | 16 | 32 | 64, chunk]) + b)
        # After a 3.8 None handshake (version, security type, ClientInit) come the messages.
        if b[:13] == b"RFB 003.008\n\x01" and len(b) > 14:
            write("client_messages", name, b[14:])

    # The Proxy: a recorded Viewer's handshake and messages, then a recorded Target's bytes.
    for target in ("tigervnc-3.8-none-copyrect", "tigervnc-3.8-vncauth", "tigervnc-3.8-none-tight-jpeg"):
        server = (FIXTURES / f"{target}.server.bin").read_bytes()
        viewer = (FIXTURES / f"{target}.client.bin").read_bytes()
        flags = 2 | (4 if "vncauth" in target else 0)
        data = bytearray([flags])
        for leg, b in ((0, viewer), (0x80, server)):
            for i in range(0, len(b), 127):
                piece = b[i:i + 127]
                data += bytes([leg | len(piece)]) + piece
        write("proxy_session", target, bytes(data))
        write("proxy_session", f"{target}-tap", bytes([data[0] | 16]) + bytes(data[1:]))

    write("record_replay", "tigervnc-copyrect", recording_from(
        (FIXTURES / "tigervnc-3.8-none-copyrect.server.bin").read_bytes()))


def recording_from(server):
    """A Recording of a 3.8 None session whose updates hold Raw (32 bpp) and CopyRect only, one
    entry per message as the Proxy writes them, after the Viewer's SetEncodings and request."""
    init_start = 18
    init_end = init_start + 24 + struct.unpack(">I", server[init_start + 20:init_start + 24])[0]
    init = server[init_start:init_end]
    width, height = struct.unpack(">HH", init[:4])
    out = bytearray(b"JUSTVNC-RECORDING" + struct.pack(">HI", 1, len(init)) + init)
    time = 0

    def entry(direction, data):
        nonlocal time
        time += 1000
        out.extend(struct.pack(">BQI", direction, time, len(data)) + data)

    entry(1, struct.pack(">BBH2i", 2, 0, 2, 1, 0))
    entry(1, struct.pack(">BBHHHH", 3, 0, 0, 0, width, height))
    at = init_end
    while at < len(server):
        count = struct.unpack(">H", server[at + 2:at + 4])[0]
        header, at = server[at:at + 4], at + 4
        for n in range(count):
            w, h, enc = struct.unpack(">HHi", server[at + 4:at + 12])
            size = 12 + (w * h * 4 if enc == 0 else 4)
            entry(0, (header if n == 0 else b"") + server[at:at + size])
            at += size
    return bytes(out)


if __name__ == "__main__":
    main()
