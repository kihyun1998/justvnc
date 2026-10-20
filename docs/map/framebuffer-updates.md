# Framebuffer updates

How a client session turns the server's messages after ServerInit into pixels. Framing: `justvnc_proto::server_message::ServerMessageReader`. Decoding: `justvnc_decode::Decoder`. Events and the full-screen copy: `justvnc_client::{ClientSession, Framebuffer}`. Framing and decoding are separate on purpose (ADR-0004).

## What the session sends by itself

Right after ServerInit, before the host can ask for anything: SetPixelFormat, when `ClientConfig::pixel_format` is set, then SetEncodings (DesktopSize, Raw). SetPixelFormat goes first because the colour map and pixel layout must be settled before any update is requested (RFC 6143 §7.5.1). FramebufferUpdateRequest is never sent automatically; the host calls `request_update`. The pixel format cannot be changed later in a session: updates already in flight would arrive in the old format, and nothing in the protocol marks where the new one starts.

## Rules taken from the specs

- **Raw** is `width * height * bytes-per-pixel` bytes, rows top to bottom (RFC 6143 §7.7.1). Every client must accept it even when it did not ask for it.
- **DesktopSize** (-223) is a rectangle with no data. Its x and y are ignored; width and height are the new size. The RFC asks clients to keep the top-left part of the old framebuffer, which `Framebuffer` does.
- **Other server messages** can arrive between updates. Bell and ServerCutText are framed and reported (see `input-and-clipboard.md`). SetColourMapEntries (1), and any other type, stops framing with `UnsupportedMessage` instead of being skipped: each message has its own length, and skipping a byte count the reader does not know desynchronises everything after it.
- **Colour-mapped formats** are refused (`UnsupportedPixelFormat`), whether configured or the server's own. They need SetColourMapEntries, which is not handled.

## Untrusted sizes

Everything is checked before the bytes it announces are waited for:

- the framebuffer size from ServerInit and from DesktopSize, against `max_framebuffer_width` / `max_framebuffer_height`. The default of 8192 is vnc-rs's limit too.
- the rectangle count, against `max_rectangles_per_update`. The default of 16384 is **not measured**: no real server's rectangle counts have been recorded. Revisit it once Hextile- or tile-based servers are recorded.
- every Raw rectangle lies inside the current framebuffer. Without this check, a single 65535x65535 rectangle would make the reader wait for 17 GB.

## After an error

A session that has returned an error keeps returning it. In the message phase this would hold anyway, because the offending bytes stay buffered. The case that needs it is a refused pixel format: the handshake has finished, so without the stored error the next `receive` would quietly succeed.

## Fixtures and the frame test

`tigervnc-3.8-none-retro-update` records TigerVNC 1.13.1 started with `GEOMETRY=48x24 XVNC_ARGS=-retro` answering a full update request. It arrives as five Raw rectangles. `-retro` draws a diagonal stipple: a pixel is white exactly when `x % 4 == [3, 1, 2, 0][y % 4]`. This rule was read from a probe written independently of justvnc. Because Cursor was not requested, Xvnc also draws its 16x16 X cursor into the framebuffer at the screen centre, so tests skip that square. The CI interop test checks the same rule on a 1024x768 frame, above the cursor.
