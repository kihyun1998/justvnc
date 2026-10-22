# Fuzzing

The fuzz targets in `fuzz/` feed arbitrary bytes to what reads a peer's bytes. They look for panics, hangs and allocations past the limits, and each checks a property a crash alone would not show.

`fuzz/` is its own workspace, not a member of the repository's: it builds on nightly with sanitizers, so the workspace gates never build it. CI checks its format and lints in the fuzz job.

## What each target checks

- **`server_messages`** frames a stream with `ServerMessageReader`. The pixel format, framebuffer size and LastRect come from the input too, because a Proxy frames with whatever format its Viewer chose. Two readers run side by side: one always sees the rest of the stream, the other first sees only half of it. The half must either frame the same piece or return `Ok(None)` and leave the reader as if it had not been called. A reader that kept state from a call that waited would frame the rest of the stream wrongly, and only once bytes arrived split at that point.
- **`client_handshake`** feeds the same server bytes all at once and in chunks of a size the input picks. Both must end with the same result and send the same bytes.
- **`client_session`** runs `ClientSession` with the `jpeg` feature on, so ZRLE, Tight and the JPEG decoder all see the input. It also checks that an error sticks (`framebuffer-updates.md`, "After an error").

- **`client_messages`** frames the Viewer's messages with `ClientMessageReader`, with the same half-the-bytes check as `server_messages`.
- **`server_session`** runs `ServerSession` against a 64x32 framebuffer whose `read` asserts that every rectangle it is asked for lies inside it. The input picks the version, the security type, whether authentication is accepted, and whether a change is reported after each chunk. It checks that an error sticks.

- **`proxy_session`** feeds a `ProxySession` chunks the input marks as from the Viewer or from the Target, deciding authentication and the Target leg from a flags byte, and checks that an error sticks on both legs. Seeded with a recorder's handshake and a TigerVNC recording; two minutes ran about 20 000 inputs without a finding.

- **`record_replay`** reads arbitrary bytes as a Recording and replays them to the end, checking that replay stops at its first error. Its seed is TigerVNC's CopyRect session written as the Proxy would, one entry per message (`seed_corpus.py`); it replays without error.

## Time bound per input

CI passes `-timeout=10`. A decoder that stops making progress hangs instead of failing: with the ZRLE inflate limit removed (#7), the inflate loop never ends. The timeout turns that into a finding. That mutation, run against `client_session`, was reported as a timeout after 14 seconds.

## Measured

Each target was checked by planting a bug and watching it found within 150 seconds: Tight fill framed without its TPIXEL (out-of-bounds slice), the update header kept after waiting (`server_messages` split assertion), handshake bytes past a step dropped (`client_handshake` assertion), the session error not stored, and the ZRLE inflate limit removed (timeout). The two server-side targets (#11) were checked the same way: a SetEncodings framed before all its bytes arrived (`client_messages` split assertion) and an update request not clipped to the framebuffer (`server_session`'s bounds assertion in `read`). In two-minute runs, `client_messages` ran about 21 000 inputs a second and `server_session` about 2 200.

Found by the first five-minute run of `client_session`: a server pixel format with a shift past the pixel's width (`framebuffer-updates.md`, "Invalid formats"). Kept as `crates/justvnc-client/tests/fixtures/fuzz-tight-blue-shift-38.server.bin`, the minimized input without its two flag bytes.

`client_session` runs at about 75 inputs a second against about 40 000 for the other two. Its seeds are whole recordings of up to 256 KB, and inflating them under ASan costs about 65 ms each. A one-minute CI run is a smoke test for it, not a search.

## Windows

`cargo fuzz run` builds on Windows but the targets do not start: with ASan, the `clang_rt.asan_dynamic` DLL Visual Studio 2019 ships does not match nightly's LLVM (`STATUS_DLL_INIT_FAILED`), and with `-s none` linking fails. Run them in a Linux container with `rustlang/rust:nightly` and `cargo-fuzz` installed, with the repository mounted. The Podman machine has 2 GiB, so pass `-rss_limit_mb=1536`.
