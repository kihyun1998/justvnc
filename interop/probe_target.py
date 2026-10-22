"""Finds out what a VNC server is and which encodings it sends, and prints a report.

Usage: python probe_target.py HOST PORT [--password PW]

Without --password, it asks for one when the server offers only VNC authentication. Nothing is
written to disk: no pixels, and no challenge or response, are kept or printed. The report holds
the server's ProtocolVersion string, the security types it offers, ServerInit (size, pixel
format, desktop name: redact the name before sharing it if it is sensitive), the encoding the
server answers each encoding offer with, and the pseudo-encodings and extra messages it answers
with when they are offered.

Each encoding is probed on a connection of its own: SetEncodings with that encoding only, then a
full FramebufferUpdateRequest for a 16x16 area. A server that does not support the encoding
must still answer, in Raw, so the first rectangle's encoding is the answer. The script stops
reading there and closes the connection; it never parses pixel data. The shared flag is set, so
other Viewers stay connected.

Needs pycryptodome for VNC authentication (`pip install pycryptodome`).
"""
import argparse
import getpass
import socket
import struct

from Crypto.Cipher import DES

ENCODINGS = [
    (7, "Tight"),
    (16, "ZRLE"),
    (15, "TRLE"),
    (5, "Hextile"),
    (2, "RRE"),
    (4, "CoRRE"),
    (6, "Zlib"),
    (17, "ZYWRLE"),
    (21, "JPEG"),
    (50, "Open H.264"),
]
PSEUDO = [
    (-223, "DesktopSize"),
    (-224, "LastRect"),
    (-232, "PointerPos"),
    (-239, "Cursor"),
    (-240, "XCursor"),
    (-258, "QEMU Extended Key Event"),
    (-307, "DesktopName"),
    (-308, "ExtendedDesktopSize"),
    (-309, "xvp"),
    (-312, "Fence"),
    (-313, "ContinuousUpdates"),
    (-314, "Cursor With Alpha"),
    (-1063131698, "Extended Clipboard"),
]
SECURITY = {
    1: "None",
    2: "VNC Authentication",
    5: "RA2",
    6: "RA2ne",
    13: "RA2 two-step",
    16: "Tight",
    17: "Ultra",
    18: "TLS",
    19: "VeNCrypt",
    20: "SASL",
    21: "MD5",
    22: "xvp",
    30: "Apple Diffie-Hellman",
    35: "Apple (macOS)",
    113: "MSLogonII",
    129: "RA2-256",
    130: "RA2ne-256",
    133: "RA2 two-step-256",
}
NAMES = dict(ENCODINGS + PSEUDO + [(0, "Raw"), (1, "CopyRect")])


def vnc_auth_response(password, challenge):
    key = password.encode("latin-1")[:8].ljust(8, b"\0")
    key = bytes(int(f"{b:08b}"[::-1], 2) for b in key)
    return DES.new(key, DES.MODE_ECB).encrypt(challenge)


class Refused(Exception):
    pass


class Connection:
    """One RFB connection through ServerInit, as a client with the shared flag set."""

    def __init__(self, host, port, password):
        self.sock = socket.create_connection((host, port), timeout=10)
        self.version = self.read(12).decode("latin-1").strip()
        major, minor = int(self.version[4:7]), int(self.version[8:11])
        self.minor = 8 if (major, minor) >= (3, 8) else 7 if (major, minor) == (3, 7) else 3
        self.sock.sendall(f"RFB 003.00{self.minor}\n".encode())
        if self.minor == 3:
            chosen = self.u32()
            if chosen == 0:
                raise Refused(self.reason())
            self.offered = [chosen]
        else:
            count = self.read(1)[0]
            if count == 0:
                raise Refused(self.reason())
            self.offered = list(self.read(count))
            if 1 in self.offered:
                chosen = 1
            elif 2 in self.offered:
                chosen = 2
            else:
                raise Refused("no security type this probe can do (None or VNC Authentication)")
            self.sock.sendall(bytes([chosen]))
        if chosen == 2:
            if password is None:
                raise Refused("VNC Authentication needs --password")
            self.sock.sendall(vnc_auth_response(password, self.read(16)))
        if chosen == 2 or self.minor == 8:
            if self.u32() != 0:
                raise Refused(self.reason() if self.minor == 8 else "authentication failed")
        self.sock.sendall(b"\x01")
        header = self.read(24)
        self.width, self.height = struct.unpack(">HH", header[:4])
        self.format = header[4:20]
        self.name = self.read(struct.unpack(">I", header[20:24])[0])

    def read(self, n):
        data = b""
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise EOFError("the server closed the connection")
            data += chunk
        return data

    def u32(self):
        return struct.unpack(">I", self.read(4))[0]

    def reason(self):
        return self.read(self.u32()).decode("latin-1", "replace")

    def request(self, encodings, incremental=False):
        self.sock.sendall(struct.pack(f">BBH{len(encodings)}i", 2, 0, len(encodings), *encodings))
        w, h = min(16, self.width), min(16, self.height)
        self.sock.sendall(struct.pack(">BBHHHH", 3, int(incremental), 0, 0, w, h))

    def close(self):
        self.sock.close()


def describe_format(f):
    bpp, depth, big, true_colour, rmax, gmax, bmax, rs, gs, bs = struct.unpack(">BBBBHHHBBB", f[:13])
    colour = (f"true colour, max {rmax}/{gmax}/{bmax}, shifts {rs}/{gs}/{bs}"
              if true_colour else "colour-mapped")
    return f"{bpp} bpp, depth {depth}, {colour}, {'big' if big else 'little'}-endian"


def skip_cut_text(conn):
    """Skips a ServerCutText. A negative length is the Extended Clipboard form."""
    conn.read(3)
    length = struct.unpack(">i", conn.read(4))[0]
    conn.read(abs(length))


def first_rectangle(conn):
    """Reads server messages until the first rectangle of a FramebufferUpdate; returns its
    encoding and the other message types seen before it."""
    others = []
    while True:
        kind = conn.read(1)[0]
        if kind == 0:
            conn.read(1)
            count = struct.unpack(">H", conn.read(2))[0]
            if count == 0:
                return None, others
            return struct.unpack(">HHHHi", conn.read(12))[4], others
        others.append(kind)
        if kind == 2:  # Bell
            continue
        if kind == 3:
            skip_cut_text(conn)
        elif kind == 150:  # EndOfContinuousUpdates
            continue
        elif kind == 248:  # ServerFence
            conn.read(3 + 4)
            conn.read(conn.read(1)[0])
        else:
            raise SystemExit(f"server message type {kind} not understood; stopping")


def pseudo_answers(conn, offered):
    """Offers `offered` with Raw and lists what the first update holds."""
    conn.request([0] + offered)
    seen, others = [], []
    bpp = conn.format[0] // 8
    while True:
        kind = conn.read(1)[0]
        if kind != 0:
            others.append(kind)
            if kind == 3:
                skip_cut_text(conn)
            elif kind == 248:
                conn.read(7)
                conn.read(conn.read(1)[0])
            elif kind not in (2, 150):
                raise SystemExit(f"server message type {kind} not understood; stopping")
            continue
        conn.read(1)
        count = struct.unpack(">H", conn.read(2))[0]
        n = 0
        while count == 0xFFFF or n < count:
            n += 1
            _, _, w, h, enc = struct.unpack(">HHHHi", conn.read(12))
            seen.append(enc)
            if enc == 0:
                conn.read(w * h * bpp)
            elif enc == -239:  # Cursor: pixels, then a bitmask
                conn.read(w * h * bpp + (w + 7) // 8 * h)
            elif enc == -240:  # XCursor: two colours and two bitmasks, if not empty
                if w and h:
                    conn.read(6 + 2 * ((w + 7) // 8 * h))
            elif enc == -308:  # ExtendedDesktopSize: screens
                screens = conn.read(4)[0]
                conn.read(16 * screens)
            elif enc == -307:  # DesktopName
                conn.read(conn.u32())
            elif enc == -224:
                break
            elif enc in (-223, -232, -258):
                pass
            else:
                # Its data cannot be skipped without decoding it (Cursor With Alpha holds
                # encoded pixels); what came so far is the answer.
                return seen + [("rest not read", enc)], others
        return seen, others


def main():
    p = argparse.ArgumentParser()
    p.add_argument("host")
    p.add_argument("port", type=int)
    p.add_argument("--password")
    a = p.parse_args()
    password = a.password

    def connect():
        nonlocal password
        try:
            return Connection(a.host, a.port, password)
        except Refused as e:
            if password is None and "needs --password" in str(e):
                password = getpass.getpass("VNC password: ")
                return Connection(a.host, a.port, password)
            raise

    conn = connect()
    print(f"ProtocolVersion: {conn.version!r} (answered 3.{conn.minor})")
    print("Security types offered:",
          ", ".join(f"{t} ({SECURITY.get(t, 'unknown')})" for t in conn.offered))
    print(f"Desktop: {conn.width}x{conn.height}, name {conn.name.decode('utf-8', 'replace')!r}")
    print(f"Pixel format: {describe_format(conn.format)}")
    conn.close()

    print("Encodings (offered alone; the server's answer):")
    for encoding, name in ENCODINGS:
        conn = connect()
        conn.request([encoding])
        answer, others = first_rectangle(conn)
        conn.close()
        verdict = "supported" if answer == encoding else "not supported"
        got = "no rectangle" if answer is None else NAMES.get(answer, str(answer))
        extra = f"; also messages {others}" if others else ""
        print(f"  {name:12} ({encoding:>4}): {verdict}, answered with {got}{extra}")

    # A pseudo-rectangle whose data cannot be skipped ends the reading; offer the rest again
    # without it.
    offered = [e for e, _ in PSEUDO]
    answered, others = set(), []
    while True:
        conn = connect()
        seen, more = pseudo_answers(conn, offered)
        conn.close()
        others += more
        answered |= {e for e in seen if isinstance(e, int) and e != 0}
        stopped = [e for e in seen if isinstance(e, tuple)]
        if not stopped:
            break
        unreadable = stopped[0][1]
        answered.add(unreadable)
        offered.remove(unreadable)
    print("Pseudo-encodings answered in the first update:",
          ", ".join(f"{NAMES.get(e, e)} ({e})" for e in sorted(answered)) or "none")
    print("Other messages before it:", sorted(set(others)) or "none",
          "(3 ServerCutText, 150 EndOfContinuousUpdates, 248 Fence)")
    print("Not detectable this way: DesktopSize, LastRect, CopyRect (they are only sent when the"
          " screen changes)")


if __name__ == "__main__":
    try:
        main()
    except Refused as e:
        raise SystemExit(f"refused: {e}")
