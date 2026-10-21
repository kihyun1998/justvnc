# Development

## Gates

The same checks CI runs (`.github/workflows/ci.yml`):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p justvnc-proto --target wasm32-unknown-unknown
```

## Interop tests

Interop tests connect to a real VNC server and are `#[ignore]`d by default. The servers are the containers defined under `interop/` (Docker or Podman). TigerVNC runs twice, without and with a password:

```sh
podman build -t justvnc-tigervnc interop/tigervnc
podman run -d --rm --name jv-tiger -e XVNC_ARGS=-retro -p 15900:5900 justvnc-tigervnc
podman run -d --rm --name jv-tiger-auth -e VNC_PASSWORD=justvnc-secret -p 15901:5900 justvnc-tigervnc
JUSTVNC_TIGERVNC_ADDR=127.0.0.1:15900 JUSTVNC_TIGERVNC_AUTH_ADDR=127.0.0.1:15901 \
JUSTVNC_TIGERVNC_EXEC="podman exec jv-tiger" \
  cargo test -p justvnc-client --features tokio --test interop -- --ignored --test-threads=1
podman rm -f jv-tiger jv-tiger-auth
```

The Proxy's interop tests take the same variables and containers:

```sh
JUSTVNC_TIGERVNC_ADDR=127.0.0.1:15900 JUSTVNC_TIGERVNC_AUTH_ADDR=127.0.0.1:15901 \
JUSTVNC_TIGERVNC_EXEC="podman exec jv-tiger" \
  cargo test -p justvnc-proxy --features tokio --test interop -- --ignored --test-threads=1
```

The tests share one X server (its clipboard and pointer), so they run one at a time. `JUSTVNC_TIGERVNC_EXEC` is how a test runs X tools (`xsel`, `xev`, `xkbbell`) inside the container.

The container takes `GEOMETRY` (default `1024x768`) and `XVNC_ARGS`; `-retro` draws a known stipple the frame test checks.

Ports 15900/15901 avoid a VNC server that may already be listening on 5900 locally (see `docs/map/rfb-handshake.md`). CI uses 5900/5901.

## Fuzzing

The fuzz targets live in `fuzz/`, a crate outside the workspace that builds on nightly with `cargo-fuzz`. CI runs each for a minute with the recorded fixtures as seeds:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
python3 fuzz/seed_corpus.py
cd fuzz && cargo +nightly fuzz run server_messages corpus/server_messages -- -max_total_time=60 -timeout=10
```

The targets are `server_messages` and `client_messages` (framing in each direction), `client_handshake`, `client_session` (the whole client, decoding included), `server_session` and `proxy_session` (both legs, interleaved). On Windows, run them in a Linux container; see `docs/map/fuzzing.md`. A crash is fixed and its input kept as an ordinary test in the crate it was found in.

`crates/justvnc-proto/tests/raised_limits.rs` only fails on a 32-bit target. CI runs it on i686: `cargo test -p justvnc-proto --target i686-unknown-linux-gnu`.

## Example Viewer

```sh
cargo run -p justvnc-client --features tokio --example viewer -- 127.0.0.1:15900
```

`JUSTVNC_PASSWORD` sets the VNC password. It syncs the local clipboard both ways, so running it replaces whatever the clipboard held.

## Example server

```sh
cargo run -p justvnc-server --features tokio --example pattern -- 127.0.0.1:15950
```

It serves a 640x480 gradient with a white square moving by CopyRect and prints the Viewer's input. The `r` key switches the desktop to 800x600 and back. `JUSTVNC_PASSWORD` turns on VNC authentication.

## Example Proxy

```sh
JUSTVNC_TOKEN=token123 cargo run -p justvnc-proxy --features tokio --example proxy -- 127.0.0.1:15952 127.0.0.1:15900
```

Viewers connect to 127.0.0.1:15952; with `JUSTVNC_TOKEN` set they authenticate with it. `JUSTVNC_TARGET_PASSWORD` is the Target's own password. `JUSTVNC_BLOCK_CLIPBOARD=1` drops clipboard text both ways; each session's lifecycle events are printed.

## Recording fixtures

Session tests are driven by bytes recorded from real servers. The recorder needs `pycryptodome` (`pip install pycryptodome`) for VNC authentication:

```sh
python interop/record_handshake.py HOST PORT crates/justvnc-client/tests/fixtures/<server>-<version>-<case> \
  [--version 3.3|3.7|3.8] [--password PW] [--update]
```

It writes `<prefix>.server.bin` and `<prefix>.client.bin`. `--update` also asks for one full Raw update. Keep its screen small (`GEOMETRY=48x24`); a 1024x768 update is 3 MB. For a 3.3 or 3.7 server, use `interop/x11vnc`, which takes `RFB_VERSION` and `VNC_PASSWORD`:

```sh
podman build -t justvnc-x11vnc interop/x11vnc
podman run -d --rm --name jv-x11 -e RFB_VERSION=3.3 -p 15910:5900 justvnc-x11vnc
```

For updates, `interop/record_updates.py` takes a list of steps (`full`, `incremental`, `run:CMD`) and records each update the server sends:

```sh
python interop/record_updates.py 127.0.0.1 15930 <prefix> --exec "podman exec jv-rec" \
  "run:(xlogo -geometry 40x40+8+8 &); sleep 1" full "run:xdotool search --class xlogo windowmove 50 30" incremental full
```

By default it offers the session's encodings (DesktopSize, LastRect, CopyRect, Tight, ZRLE, Raw); `--encodings 0` asks for Raw only, which gives a reference picture for a compressed recording. The ZRLE fixture was made like this, on x11vnc (`GEOMETRY` sets its screen size):

```sh
python interop/make_zrle_image.py zrle-test.xwd
podman run -d --rm --name jv-rec -e GEOMETRY=256x128 -p 15930:5900 justvnc-x11vnc
podman cp zrle-test.xwd jv-rec:/tmp/zrle-test.xwd
python interop/record_updates.py 127.0.0.1 15930 <prefix> --exec "podman exec jv-rec" \
  "run:(xwud -in /tmp/zrle-test.xwd -geometry +0+0 &); sleep 1.5" full full
python interop/record_updates.py 127.0.0.1 15930 <prefix>-reference-raw --exec "podman exec jv-rec" --encodings 0 full
python interop/zrle_census.py <prefix>.server.bin
```

`zrle_census.py` lists which ZRLE subencodings a recording holds. The ZRLE recording offers `--encodings=-223,1,16,0`, since Tight is now ahead of ZRLE in the default list.

The Tight fixture comes from Xtightvnc (`interop/tightvnc`), with the windows from `make_tight_images.py` opened beside the ZRLE image:

```sh
python interop/make_zrle_image.py .; python interop/make_tight_images.py .
podman build -t justvnc-tightvnc interop/tightvnc
podman run -d --rm --name jv-rec -e GEOMETRY=512x128 -p 15940:5900 justvnc-tightvnc
for f in zrle-test tight-mono tight-indexed tight-noise; do podman cp $f.xwd jv-rec:/tmp/$f.xwd; done
python interop/record_updates.py 127.0.0.1 15940 <prefix> --exec "podman exec jv-rec" \
  "run:(xwud -in /tmp/zrle-test.xwd -geometry +0+0 &); sleep 1.5" full \
  "run:(xwud -in /tmp/tight-mono.xwd -geometry +256+0 &); sleep 1" incremental \
  "run:(xwud -in /tmp/tight-indexed.xwd -geometry +320+0 &); sleep 1" incremental \
  "run:(xwud -in /tmp/tight-noise.xwd -geometry +384+0 &); sleep 1" incremental
python interop/record_updates.py 127.0.0.1 15940 <prefix>-reference-raw --exec "podman exec jv-rec" --encodings 0 full
python interop/tight_census.py <prefix>.server.bin
```

The JPEG fixture comes from TigerVNC with a quality level offered (`-24` is level 8), and its expected pixels from Pillow:

```sh
python interop/record_updates.py 127.0.0.1 15930 <prefix> --exec "podman exec jv-rec" --encodings=-223,-224,1,7,16,0,-24   "run:(xwud -in /tmp/zrle-test.xwd -geometry +0+0 &); sleep 1.5" full full
python interop/tight_jpeg_reference.py <prefix>.server.bin <prefix>.libjpeg.bin [<raw-reference>.server.bin]
```

The JPEG tests run only with the feature: `cargo test --workspace --all-features`. CI also runs `cargo test --workspace` to cover the session without it.

Name a fixture after the server that produced it, and note where it came from in the map note for that area.

The pixel format fixtures are TigerVNC showing colour ramps, recorded once per format with `--pixel-format` (bits-per-pixel, depth, big-endian, then the maxes and shifts):

```sh
python interop/make_ramp_image.py ramp.xwd
podman run -d --rm --name jv-ramp -e GEOMETRY=256x16 -p 15960:5900 justvnc-tigervnc
podman cp ramp.xwd jv-ramp:/tmp/ramp.xwd
python interop/record_updates.py 127.0.0.1 15960 <prefix>-xrgb --exec "podman exec jv-ramp" --encodings 0 \
  "run:(xwud -in /tmp/ramp.xwd -geometry +0+0 &); sleep 1.5" full
python interop/record_updates.py 127.0.0.1 15960 <prefix>-rgb565 --exec "podman exec jv-ramp" --encodings 0 \
  --pixel-format=16,16,0,31,63,31,11,5,0 full
```

To record a real Viewer, put `interop/record_relay.py` between it and a server. It writes `<prefix>.viewer.bin` and `<prefix>.server.bin` when the Viewer disconnects:

```sh
python interop/record_relay.py 15951 127.0.0.1 15950 <prefix>
```

Then connect the Viewer to `127.0.0.1::15951`. A Viewer's fixtures live in `crates/justvnc-server/tests/fixtures/`.
