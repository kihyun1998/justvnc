# Framebuffer updates

How a client session turns the server's messages after ServerInit into pixels. Framing: `justvnc_proto::server_message::ServerMessageReader`. Decoding: `justvnc_decode::Decoder`. Events and the full-screen copy: `justvnc_client::{ClientSession, Framebuffer}`. Framing and decoding are separate on purpose (ADR-0004).

## What the session sends by itself

Right after ServerInit, before the host can ask for anything: SetPixelFormat, when `ClientConfig::pixel_format` is set, then SetEncodings (DesktopSize, CopyRect, Raw). Each encoding added later goes into this list (`ENCODINGS` in the client session) at its preference position, or no server will send it. SetPixelFormat goes first because the colour map and pixel layout must be settled before any update is requested (RFC 6143 §7.5.1). FramebufferUpdateRequest is never sent automatically; the host calls `request_update`. The pixel format cannot be changed later in a session: updates already in flight would arrive in the old format, and nothing in the protocol marks where the new one starts.

## Rules taken from the specs

- **Raw** is `width * height * bytes-per-pixel` bytes, rows top to bottom (RFC 6143 §7.7.1). Every client must accept it even when it did not ask for it.
- **CopyRect** (1) carries only a source position (`src-x`, `src-y`, 4 bytes; RFC 6143 §7.7.2). It is framed straight to `Event::Copy` without going through the decoder, because it has no pixels. Both the destination and the source must lie inside the framebuffer. A full (non-incremental) update never uses it (§7.5.3). Servers should not copy from pixels updated earlier in the same update, so applying rectangles in order is enough.
- **Copying an overlapping area.** Source and destination can overlap, e.g. a window dragged a few pixels. `Framebuffer::copy` reads the source as it was before the copy. Rows run bottom-up when the copy moves down and top-down otherwise, so no source row is overwritten before it is read. Within a row, `copy_within` handles the overlap.
- **ZRLE** (16) is a U32 length and that many bytes of zlib data (RFC 6143 §7.7.6). Framing reads the length and nothing else, so `justvnc-proxy` never inflates. Decoding (`justvnc_decode`'s `zrle.rs`) keeps **one zlib stream for the whole connection**: every rectangle continues the previous one's dictionary, so rectangles must be decoded strictly in order, and a decoder is never reset between them. The inflated data is 64x64 tiles, each starting with a subencoding: 0 raw, 1 solid, 2–16 packed palette (1/2/4-bit indices, most significant bit first, rows padded to a byte), 128 plain RLE, 130–255 palette RLE. 127 and 129 (palette reuse) are TRLE-only and refused. A run length is one more than the sum of its bytes, ending at the first byte that is not 255.
- **CPIXEL.** A ZRLE pixel is 3 bytes when the format is true colour, 32 bpp, depth 24 or less, and the colour bits fit the low 3 bytes (every `max << shift` < 2^24) or the high 3 bytes (every shift > 7) (§7.7.5). The RFC does not say *which* 3 of the 4 wire bytes are sent. libvncserver's encoder (`src/libvncserver/zrle.c`) sends the first three when the colour is in the low bytes of a little-endian pixel or the high bytes of a big-endian one, and the last three otherwise. The decoder follows that. The bytes outside the colour bits come back as 0.
- **DesktopSize** (-223) is a rectangle with no data. Its x and y are ignored; width and height are the new size. The RFC asks clients to keep the top-left part of the old framebuffer, which `Framebuffer` does.
- **Other server messages** can arrive between updates. Bell and ServerCutText are framed and reported (see `input-and-clipboard.md`). SetColourMapEntries (1), and any other type, stops framing with `UnsupportedMessage` instead of being skipped: each message has its own length, and skipping a byte count the reader does not know desynchronises everything after it.
- **Colour-mapped formats** are refused (`UnsupportedPixelFormat`), whether configured or the server's own. They need SetColourMapEntries, which is not handled.

## Untrusted sizes

Everything is checked before the bytes it announces are waited for:

- the framebuffer size from ServerInit and from DesktopSize, against `max_framebuffer_width` / `max_framebuffer_height`. The default of 8192 is vnc-rs's limit too.
- the rectangle count, against `max_rectangles_per_update`. The default of 16384 is **not measured**: no real server's rectangle counts have been recorded. Revisit it once Hextile- or tile-based servers are recorded.
- every Raw and ZRLE rectangle, and both sides of every CopyRect, lies inside the current framebuffer.
- the ZRLE length, against `max_compressed_length` (default 64 MiB, vnc-rs's figure).
- what one ZRLE rectangle inflates to. Inflating stops with `Corrupt` past the most a valid rectangle can need: every tile a 127-colour palette, and then every pixel either a raw CPIXEL or a one-pixel run (one CPIXEL and one length byte). A few compressed bytes can otherwise inflate to gigabytes. A run that goes past its tile, or a palette index outside the palette, is also `Corrupt`. Without this check, a single 65535x65535 rectangle would make the reader wait for 17 GB.

## After an error

A session that has returned an error keeps returning it. In the message phase this would hold anyway, because the offending bytes stay buffered. The case that needs it is a refused pixel format: the handshake has finished, so without the stored error the next `receive` would quietly succeed.

## Fixtures and the frame test

`tigervnc-3.8-none-retro-update` records TigerVNC 1.13.1 started with `GEOMETRY=48x24 XVNC_ARGS=-retro` answering a full update request. It arrives as five Raw rectangles. `-retro` draws a diagonal stipple: a pixel is white exactly when `x % 4 == [3, 1, 2, 0][y % 4]`. This rule was read from a probe written independently of justvnc. Because Cursor was not requested, Xvnc also draws its 16x16 X cursor into the framebuffer at the screen centre, so tests skip that square. The CI interop test checks the same rule on a 1024x768 frame, above the cursor.

`tigervnc-3.8-none-copyrect` comes from `interop/record_updates.py` on TigerVNC at `GEOMETRY=120x80 XVNC_ARGS=-retro`. An `xlogo` window is opened at 40x40+8+8, then three updates are recorded: a full one, an incremental one after `xdotool ... windowmove 50 30`, and another full one. The incremental update holds four CopyRects and two Raw rectangles. Because a full update never uses CopyRect, the third update is the server's own picture of the result. It is the expected value for the first two applied in order, so nothing had to be computed by hand. `xlogo` is used because it is static; `xclock` redraws every second, and its two full updates would differ. The window does not overlap its old position, so overlap is covered by `Framebuffer` tests instead.

`x11vnc-3.8-none-zrle` holds two full ZRLE updates from x11vnc 0.9.16 at `GEOMETRY=256x128`, showing `interop/make_zrle_image.py`'s image with `xwud`. The image's 64x64 tiles are built to make an encoder reach for each subencoding. `interop/zrle_census.py` confirms the recording holds every one of them (raw 6, solid 2, packed palette 2, plain RLE 2, palette RLE 4) across one zlib stream. `x11vnc-3.8-none-zrle-reference-raw` is the same screen over a second connection that asked for Raw only. It is the expected value, compared on the colour bits only, because x11vnc's Raw pixels carry stray non-zero bytes outside them (40 of 32768 here) that 3-byte CPIXELs cannot carry. x11vnc is used rather than TigerVNC because TigerVNC 1.13.1's `ZRLEEncoder` picks one palette per rectangle and **never** sends plain RLE. TigerVNC's ZRLE is still exercised: given the session's list, it sends every rectangle of the CI frame test as ZRLE.

The client bytes of these recordings carry the SetEncodings the session sent when they were recorded. `tigervnc-3.8-none-retro-update` was re-recorded when CopyRect joined the list, and its server bytes came out identical. Since ZRLE joined, tests cut SetEncodings out of recordings before comparing (`tests/common`), and `handshake.rs` alone pins the list. A server answers with any encoding the client offered, so a recording made with a shorter list stays a valid answer.
