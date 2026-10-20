# RFB handshake

How a session gets from TCP connect to ServerInit. Client side: `justvnc_proto::client::ClientHandshake`, wrapped by `justvnc_client::ClientSession`. VNC authentication's response: `justvnc_proto::vnc_auth`, shared with the server role and the Proxy.

## Rules taken from the specs

- **Versions.** Only 3.3, 3.7 and 3.8 exist. Anything else a server reports (Apple's 3.889, RealVNC's 4.x/5.x, the miswritten 3.5) is interpreted as 3.3 (RFC 6143 §7.1.1, Appendix A; vnc-rs does the same). The client answers with the server's version when it is 3.7 or 3.8, and with 3.3 otherwise. It never asks for more than the server offered.
- **Security type in 3.3.** The server decides and sends a U32: 0 (refused, followed by a reason), 1 (None) or 2 (VNC authentication). The client sends no security-type byte. Any other value is a protocol violation (`UnexpectedSecurityType`).
- **Security type in 3.7 and 3.8.** The server lists types; the client takes the first one, in the server's order, that it can do. VNC authentication counts only when a password is configured. If VNC authentication was offered and nothing usable was, the error is `PasswordRequired`, not `NoSupportedSecurityType`.
- **SecurityResult after None.** In 3.8 the server sends SecurityResult (U32) even for security type None. In 3.3 and 3.7 it does not, and goes straight to ServerInit. Getting this wrong reads the first four bytes of ServerInit as a status and desynchronises silently.
- **Failed authentication.** Only 3.8 follows a failed SecurityResult with a reason. 3.3 and 3.7 send the status and close, so waiting for a reason there waits forever. `SecurityFailed { reason }` is therefore an `Option`.
- **Refusal before security.** A zero security-type count (3.7, 3.8) or a zero security type (3.3) is followed by a U32 reason length and the reason; the server then closes.
- **VNC authentication key.** The password is truncated or zero-padded to 8 bytes (RFC 6143 §7.2.2). Each byte is then bit-reversed, because VNC treats the lowest bit of each byte as DES's first bit. That rule is in rfbproto ("VNC Authentication") and **not** in RFC 6143, so a client built from the RFC alone produces responses every real server rejects. The 16-byte challenge is encrypted as two independent DES blocks (ECB).
- **DES parity.** DES ignores the lowest bit of each key byte. Two keys that differ only there are the same key, so a test that changes only those bits proves nothing.
- **Desktop name encoding.** Unspecified by RFC 6143. rfbproto recommends UTF-8 and requires tolerating invalid UTF-8 without losing sync. `DesktopName` keeps the raw bytes; `to_string_lossy` is for display only.
- **PIXEL_FORMAT.** The max fields are always big-endian, whatever `big-endian-flag` says. The 3 padding bytes are ignored on receive. The flags are booleans as "non-zero": x11vnc sends 0xff for `true-colour-flag`.

## Untrusted lengths

`name-length` and `reason-length` are U32s from the peer. Both are checked against `Limits` before the bytes they announce are waited for or allocated, so a peer announcing 4 GB ends the session with `LimitExceeded` instead of growing the receive buffer.

## Fixtures

Recorded with `interop/record_handshake.py` and named `<server>-<version>-<case>`. A recording is a pair: `.server.bin` is what the server sent, `.client.bin` what the scripted client sent back. The script computes VNC authentication with pycryptodome's DES, not justvnc's, and the server's SecurityResult in the same recording shows whether that response was accepted. That makes the client bytes an independent expected value.

- `tigervnc-3.8-*`: TigerVNC 1.13.1 (Ubuntu 24.04) from `interop/tigervnc`, `-geometry 1024x768 -depth 24 -desktop justvnc-test`. `none` with no password; `vncauth` with `VNC_PASSWORD=justvnc-secret`, which is longer than 8 bytes; `vncauth-wrong-password` answered with `wrong-pw`.
- `x11vnc-*`: x11vnc 0.9.16 (Ubuntu 24.04) on Xvfb 800x600x24, from `interop/x11vnc`, `-desktop justvnc-x11vnc`, version forced with `-rfbversion`. The 3.3 and 3.7 cases come from here, because TigerVNC always offers 3.8. `3.8-vncauth-short-password` uses `abc` to cover zero padding.
- `tightvnc-3.8-loopback-refused`: a Windows TightVNC Server refusing a loopback connection before security. Its reason string comes from TightVNC's `RfbInitializer`.

## Local interop

A TightVNC Server on the developer machine may already own port 5900, and a probe there silently talks to it instead of the container. Publish the containers on other ports (`-p 15900:5900`, `-p 15901:5900`) and point `JUSTVNC_TIGERVNC_ADDR` / `JUSTVNC_TIGERVNC_AUTH_ADDR` at them. CI has no such server and uses 5900 and 5901.
