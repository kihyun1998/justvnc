# Serving a Viewer

How the server role takes a Viewer from TCP connect to served updates. The handshake is `justvnc_proto::server::ServerHandshake`; client messages are framed by `justvnc_proto::client_message::ClientMessageReader`; `justvnc_server::ServerSession` joins them to the host's `Framebuffer`, and `justvnc_server::tokio` drives it over a stream.

## Handshake

- **Version.** The server offers its configured version; the client's answer decides. A minor of 8 or more is spoken as 3.8, 7 as 3.7, and anything else as 3.3 (rfbproto: 3.5 was wrongly reported by some clients and is taken as 3.3). The result is never above what the server offered. A major other than 3 is `UnsupportedClientVersion`.
- **Security type.** Under 3.3 the server picks, and it picks the first configured type. Under 3.7 and 3.8 it lists them in order and refuses a choice it did not list (`UnexpectedSecurityType`).
- **SecurityResult.** After None it is sent only under 3.8. A rejected response is followed by a reason string only under 3.8; under 3.3 and 3.7 the server just closes.

The handshake is checked against real servers: fed the bytes the recorder sent TigerVNC 1.13.1 and x11vnc 0.9.16, configured with the version, security types and challenge each one offered, it must send exactly what that server sent (`crates/justvnc-proto/tests/server_handshake.rs`). x11vnc writes the true-colour flag of ServerInit as 0xff; justvnc writes 1, and the test compares that byte as a boolean.

## Authentication

**Maintainer's call (#11):** the decision is an event, `Event::Authenticate { challenge, response }`, answered with `accept` or `reject(reason)`, rather than a closure in `ServerConfig`. The maintainer was shown both. The event lets a decision wait on something slow, such as a Proxy asking a broker whether a one-time token is valid, without blocking a thread. The tokio driver takes an async closure and handles the event inside, so a fixed-password server is still one line with `verify`.

The challenge comes from `getrandom` in `ServerSession::new`. `ServerSession::with_challenge` takes one instead: tests replay recordings with the challenge the recording holds, and a host with its own random source can use that one. The challenge must never repeat; a repeated one lets a recorded response be replayed.

VNC authentication keys DES with the first 8 bytes of the password, so a host using tokens as passwords should put 8 bytes of randomness at the start of each token.

## Updates

**Maintainer's call (#11):** a non-incremental FramebufferUpdateRequest is answered at once; an incremental one waits until the host reports a change with `damage`, and is answered with the part of the change inside the requested area. The maintainer was shown oxidecomputer/rfb's alternative, which answers every request with the whole screen; a Viewer re-requests as soon as an update arrives, so that sends the full screen in a loop even when nothing moves (3 MB a round at 1024x768).

Changes not yet sent are kept as non-overlapping rectangles (`region.rs`). A request takes the parts inside its area and leaves the rest for a later request. The first version kept one bounding rectangle and cleared it whenever part of it was sent, which silently lost a change outside a partial request; `keeps_a_change_outside_the_requested_area_for_a_later_request` pins that. Past 64 rectangles the region becomes the one rectangle that holds them, which may resend unchanged pixels but never drops changed ones. The 64 is **not measured**.

A request reaching past the framebuffer is clipped to it. A non-incremental request with nothing inside the framebuffer is answered with an update of no rectangles, so a Viewer waiting for an answer gets one.

Pixels go as Raw. Moves go as CopyRect and size changes as DesktopSize, below; no compressed encoding is sent.

## Moves (CopyRect)

The host reports a move with `copy(rect, src_x, src_y)`: the pixels now in `rect` came from (`src_x`, `src_y`). It is sent as CopyRect only when the Viewer can copy it correctly, and otherwise as a change to `rect`:

- the Viewer offered CopyRect (1) in SetEncodings;
- both rectangles lie inside the framebuffer;
- no change inside the source is waiting to be sent, because the Viewer copies from what it has, and it does not have that change yet;
- no other move is waiting. One waiting move at a time keeps the order simple; a second becomes a change.

A waiting move goes first in the next update, before any Raw rectangle, so Raw pixels sent for the same area land on top of it. A change reported after the move, even one inside its source, is fine for the same reason: the Viewer copies before it draws.

## Size changes (DesktopSize)

The host reports a possible size change with `resize`. Nothing is sent unless the size really changed: rfbproto warns that some clients loop if told of a resize that did not happen. The DesktopSize pseudo-rectangle goes alone in an update that answers the Viewer's next request (or one already waiting), then the whole framebuffer counts as changed, because rfbproto says the Viewer may discard its framebuffer on a resize. Waiting changes and a waiting move are dropped at the resize.

**Maintainer's call (#12):** a Viewer that did not offer DesktopSize (-223) cannot follow a resize, so `resize` ends the session with `Error::ViewerCannotResize`. The maintainer was shown the alternative of carrying on at the old size, clipped, with the grown part never shown.

## Pixel format

A SetPixelFormat that is not valid (`PixelFormat::is_valid`, `framebuffer-updates.md`) or is colour-mapped ends the session with `UnsupportedPixelFormat`. Any other is served: `convert.rs` rewrites the framebuffer's pixels into it.

Each colour is scaled as `(c * to_max + (from_max + 1) / 2) / from_max`, which rounds half up. rfbproto does not say how to scale. This is what TigerVNC 1.13.1 does: asked for RGB565, RGB555, BGR233 and big-endian 32-bit by `interop/record_updates.py --pixel-format` while showing `interop/make_ramp_image.py`'s ramps, every one of its 256 values per colour matched `(v * max + 128) / 255`. The usual rounding, `+ 127`, differs from it at 37 (to 5 bits) and 91 (to 3 bits). The recordings are `crates/justvnc-server/tests/fixtures/tigervnc-ramp-*.server.bin`, and the server must send exactly their bytes.

TightVNC Viewer 2.8.85 sends no SetPixelFormat at all with its default settings and takes the server's format.

## Untrusted lengths

ClientCutText is checked against `max_cut_text_length` before its bytes are waited for. SetEncodings has a U16 count, so it holds at most 256 KiB and has no limit of its own.

## The tokio driver and cancellation

`Connection::serve` and `Connection::damage` are cancel-safe, so a host can `select!` between them (the `pattern` example does). The bytes taken from the session are kept in the connection until written, and the next call writes what a cancelled one did not. Writing them with `write_all` from a local buffer, as the client driver does, would lose the unwritten part of a cancelled write and corrupt the stream. `a_serve_cancelled_while_writing_loses_no_bytes` cancels a serve blocked on a 64-byte pipe.

## A real Viewer

`crates/justvnc-server/tests/fixtures/tightvnc-viewer-3.8-vncauth.viewer.bin` is what TightVNC Viewer 2.8.85 on Windows sent the `pattern` example (VNC authentication, password `justvnc`) through `interop/record_relay.py`, while the maintainer typed, clicked, scrolled and copied text. The server side is justvnc's own output, so only its handshake is kept (`.server-handshake.bin`), for the challenge. What the Viewer sent, counted by a separate Python parse:

- ClientInit asking to share.
- SetEncodings once: Tight (7), CopyRect (1), ZRLE (16), RRE (5), ... in the order 7, 1, 16, 5, 2, 0, -26, -223, -224, -232, -239. No SetPixelFormat.
- One full request, then 1954 incremental ones, 918 pointer events, 28 key events, one ClientCutText (`"\nThe \"close"`).

`tightvnc-viewer-3.8-none-8bit.viewer.bin` is the same Viewer with its 8-bit colour option, connected to the `pattern` example without a password while the maintainer pressed `r` three times. It sent SetPixelFormat for BGR233 (8 bpp, maxes 7, 7, 3, shifts 0, 3, 6), the format TigerVNC's ramp was recorded in, and the same SetEncodings. Live, the server's side framed to its last byte at one byte a pixel: 605 Raw rectangles, 564 CopyRect and 3 DesktopSize. The maintainer saw the coarse 8-bit gradient, the square moving unbroken, and the desktop switch size. The replay test checks that the Viewer's bytes are served in BGR233; it cannot check the moves and resizes, which came from the example, not from the Viewer.
