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

The targets are `server_messages` (framing after ServerInit), `client_handshake` and `client_session` (the whole client, decoding included). On Windows, run them in a Linux container; see `docs/map/fuzzing.md`. A crash is fixed and its input kept as an ordinary test in the crate it was found in.

`crates/justvnc-proto/tests/raised_limits.rs` only fails on a 32-bit target. CI runs it on i686: `cargo test -p justvnc-proto --target i686-unknown-linux-gnu`.

## Example Viewer

```sh
cargo run -p justvnc-client --features tokio --example viewer -- 127.0.0.1:15900
```

`JUSTVNC_PASSWORD` sets the VNC password. It syncs the local clipboard both ways, so running it replaces whatever the clipboard held.

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
