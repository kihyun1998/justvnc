"""Writes starting corpora for the fuzz targets from the client role's recorded fixtures.

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


if __name__ == "__main__":
    main()
