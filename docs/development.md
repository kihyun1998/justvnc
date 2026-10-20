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

Name a fixture after the server that produced it, and note where it came from in the map note for that area.
