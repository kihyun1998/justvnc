# Recording sessions

`justvnc-record` writes and reads Recordings, and with its `replay` feature replays them (ADR-0005). A Proxy host records by handing what its Tap is told to a `RecordingWriter`: `Tap::relay_started` gives the ServerInit to start the file with, and `forwarded_to_viewer` / `forwarded_to_target` give the bytes of each message that crossed, after the Tap's verdict and the narrowing of SetEncodings. The `proxy` example does this with `JUSTVNC_RECORD`.

## What is in a Recording

The messages that crossed after both handshakes: the Target's screen updates, Bells and clipboard text, and everything the Viewer sent (keys, pointer, clipboard, SetPixelFormat, SetEncodings, update requests). **Maintainer's calls (#34):** the Viewer's input is recorded, so an audit shows what was typed as well as what was seen, and so a Recording holds any password typed during the session; and only what crossed is recorded, so a message the Tap dropped is not in it. A host that wants dropped attempts in its audit log writes them from its Tap.

No handshake byte is recorded, so a Recording holds no password, challenge or response of the handshake (`keeps_no_handshake_byte`).

## Why `relay_started` exists

In a sans-IO Proxy the Target's handshake ends inside a `receive_target` call, and the Proxy forwards the first messages in that same call. The host would only learn the ServerInit from `poll_event` afterwards, too late to have started the file. `relay_started` is called just before relaying, inside the Proxy, so a Recording never misses its first messages.

## Format, version 1

All integers big-endian.

- `JUSTVNC-RECORDING` (17 bytes), the version as U16 (1), the ServerInit's length as U32, then the ServerInit as sent.
- Then entries to the end of the file: direction as U8 (0 to the Viewer, 1 to the Target), the time as U64 microseconds since the session started, as the host measured it, the length as U32, then the bytes of exactly one message.

A FramebufferUpdate's pieces are separate entries, as the Proxy frames them: the first rectangle's entry carries the update header. An update counted out by its rectangles ends without bytes, so it has no entry of its own; replay frames that end from the rectangle before it.

## Reading untrusted Recordings

A Recording may come from anywhere. The reader checks the ServerInit length against what a server may send (24 bytes plus the default desktop name limit) and an entry's length against the longest message a session with default limits frames: a Raw rectangle filling 8192x8192 at 32 bits per pixel. The bytes are then read with `take(len).read_to_end`, which grows the buffer as bytes arrive, so a header claiming 4 GiB with nothing after it allocates nothing (`does_not_allocate_what_an_entry_only_claims`). The `record_replay` fuzz target replays arbitrary bytes.

## Replay

`Replay` cannot use `ClientSession`: that needs a handshake and keeps one pixel format. It frames with `ServerMessageReader` and decodes with `Decoder`, and applies a recorded SetPixelFormat (format and framing) and SetEncodings (LastRect) at the entry where they appear, the point where the Proxy applied them. Replay is from the start only: Tight and ZRLE carry zlib streams across rectangles.

Each entry must hold exactly one message; one that holds less or more stops replay with an error. A Recording the Proxy wrote always satisfies this; the replay tests, run on the Proxy's own Recordings of TigerVNC, would fail otherwise.

Replay needs the decoders, so it is behind `replay`, which the umbrella crate turns on with `client`. A Proxy that only records carries no decoder: `cargo tree` for `justvnc` with `proxy`, `record` and `tokio` shows no `justvnc-decode`, flate2 or zune-jpeg.

## Frames

With the `frames` feature, `Frames` turns a Recording into images and `export_frames` writes them as PNG files with an `index.csv` (#35). **Maintainer's calls (#35):** keyframes of the whole screen plus the regions each update changed, for a pipeline that already takes captured images as files; keyframes at a configurable interval, default 60 s; one index file; PNG; a library function and an example.

- A frame is taken at the end of each update (`UpdateComplete`), at that entry's time. The changed regions come from the replayed events (each `Rectangle` and `Copy`), so nothing compares pixels; each region is cropped from the screen as the update left it, so overlapping regions of one update agree.
- A keyframe is taken instead of regions at the first update, after a desktop size change (the receiver cannot draw regions onto a picture of another size), after a pixel format change (the screen starts again, black, in the new format, as the replay example does), and at the first update that changes something once the interval has passed. An update that changes nothing takes nothing, so a still screen writes no images however long it lasts.
- A Recording that ends inside an update gives what that update changed so far.
- Colours become 8-bit red, green and blue, each scaled `(c * 255 + (max + 1) / 2) / max`, the rounding the server role measured from TigerVNC (`serving-a-viewer.md`).

`index.csv` starts with the header `time_us,kind,x,y,width,height,file`, then one line per image in order: microseconds since the session started, `keyframe` or `region`, the rectangle, and the PNG's file name (numbered `000001.png` on). Drawing the images in that order, a keyframe replacing the screen and a region drawn at its x and y, rebuilds the replayed screen at every update; `tests/frames.rs` checks that for the three TigerVNC recordings and for big-endian 32-bit, reading the index and the PNGs back.

`png` is used only under `frames`, which needs `client`: a recording Proxy carries no image encoder.

## Checked

- Replaying a Proxy's Recording of TigerVNC's Raw/CopyRect, Tight JPEG with LastRect, and RGB565 recordings gives the picture the client role decodes from the same bytes (`tests/replay.rs`).
- Live: the `proxy` example in front of TigerVNC 1.13.1 recorded a session from `interop/record_updates.py`. Its entries to the Viewer were byte for byte what the recorder received after ServerInit, and all 19 replayed without error. The `frames` example turned it into 12 images: the first-update keyframe, then the regions of the logo's move.
