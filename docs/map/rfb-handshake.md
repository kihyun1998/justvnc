# RFB handshake

How a session gets from TCP connect to ServerInit. Client side: `justvnc_proto::client::ClientHandshake`, wrapped by `justvnc_client::ClientSession`.

## Rules taken from the specs

- **Versions.** Only 3.3, 3.7 and 3.8 exist. Anything else a server reports (Apple's 3.889, RealVNC's 4.x/5.x) is to be interpreted as 3.3 (RFC 6143 §7.1.1, Appendix A). A client never requests a version above the server's. Today the client accepts exactly 3.8 and refuses everything else as `UnsupportedVersion`; #3 adds 3.3 and 3.7.
- **SecurityResult after None.** In 3.8 the server sends SecurityResult (U32) even for security type None. In 3.3 and 3.7 it does not, and goes straight to ServerInit. Getting this wrong reads the first four bytes of ServerInit as a status and desynchronises silently.
- **Refusal before security.** A zero security-type count is followed by a U32 reason length and the reason; the server then closes.
- **Desktop name encoding.** Unspecified by RFC 6143. rfbproto recommends UTF-8 and requires tolerating invalid UTF-8 without losing sync. `DesktopName` keeps the raw bytes; `to_string_lossy` is for display only.
- **PIXEL_FORMAT.** The max fields are always big-endian, whatever `big-endian-flag` says. The 3 padding bytes are ignored on receive.

## Untrusted lengths

`name-length` and `reason-length` are U32s from the peer. Both are checked against `Limits` before the bytes they announce are waited for or allocated, so a peer announcing 4 GB ends the session with `LimitExceeded` instead of growing the receive buffer.

## Fixtures

Recorded with `interop/record_handshake.py`. The client side of the recording is scripted, so the files hold only server-to-client bytes.

- `tigervnc-3.8-none.bin`: TigerVNC 1.13.1 (Ubuntu 24.04 `tigervnc-standalone-server`) from `interop/tigervnc`, started with `-geometry 1024x768 -depth 24 -desktop justvnc-test -SecurityTypes None`. The tests' expected values come from those arguments.
- `tightvnc-3.8-loopback-refused.bin`: a Windows TightVNC Server refusing a loopback connection before security. Its reason string comes from TightVNC's `RfbInitializer`.

## Local interop

A TightVNC Server on the developer machine may already own port 5900, and a probe there silently talks to it instead of the container. Publish the container on another port (`-p 15900:5900`) and point `JUSTVNC_TIGERVNC_ADDR` at it. CI has no such server and uses 5900.
