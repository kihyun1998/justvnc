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

## Checked

- Replaying a Proxy's Recording of TigerVNC's Raw/CopyRect, Tight JPEG with LastRect, and RGB565 recordings gives the picture the client role decodes from the same bytes (`tests/replay.rs`).
- Live: the `proxy` example in front of TigerVNC 1.13.1 recorded a session from `interop/record_updates.py`. Its entries to the Viewer were byte for byte what the recorder received after ServerInit, and all 19 replayed without error.
