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

Interop tests connect to a real VNC server and are `#[ignore]`d by default. The server is the container defined in `interop/tigervnc` (Docker or Podman):

```sh
podman build -t justvnc-tigervnc interop/tigervnc
podman run -d --rm --name jv-tiger -p 15900:5900 justvnc-tigervnc
JUSTVNC_TIGERVNC_ADDR=127.0.0.1:15900 cargo test -p justvnc-client --features tokio --test interop -- --ignored
podman rm -f jv-tiger
```

Port 15900 avoids a VNC server that may already be listening on 5900 locally (see `docs/map/rfb-handshake.md`). CI uses 5900.

## Recording fixtures

Session tests are driven by bytes recorded from real servers. To record a 3.8 handshake with security type None:

```sh
python interop/record_handshake.py 127.0.0.1 15900 crates/justvnc-client/tests/fixtures/<server>-<version>-<case>.bin
```

Name a fixture after the server that produced it, and note where it came from in the map note for that area.
