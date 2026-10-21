# Choosing roles

The umbrella crate `justvnc` re-exports the role crates as modules, each behind a feature: `client`, `server`, `proxy` and `decode`. `justvnc::proto` is always there. `tokio` and `jpeg` forward to the role crates that are on, as weak dependency features (`justvnc-client?/tokio`), so turning on `tokio` alone pulls in no role.

**Maintainer's call (#15):** no feature is on by default. A user names the roles they use, and a binary that only serves never carries the decoders, a client never carries server code, and nobody carries Tokio by accident (spec stories 1 and 2). The maintainer was shown the alternatives: `client` by default, which needs `default-features = false` for a server-only user, and everything by default, which pulls zlib and Tokio into every build.

## What the checks catch

- Every combination of the six features is checked by `cargo hack check --feature-powerset` (64 builds). A `compile_error!` planted for one combination failed exactly the 8 builds that contain it.
- `tests/features.rs` checks that `jpeg` reaches the client role (`ClientConfig::jpeg_quality` exists only then) and the decoder (a Tight JPEG rectangle is `Corrupt` rather than `Unsupported`). The decoder's case has to run with `decode` and `jpeg` but not `client`: the client's own `jpeg` feature turns the decoder's on, so with `--all-features` a missing forward from the umbrella goes unnoticed. CI runs `--features decode,jpeg` for that.
- The README's client, server and Proxy examples are the crate's doc comment when `client`, `server`, `proxy` and `tokio` are all on, so `--all-features` compiles them as doc tests (`no_run`: they need a network). Removing the `tokio` forward of any role fails them.

## The examples' error types

The role crates' error types implement `Debug` but not `std::error::Error` or `Display`, so the README examples return each driver's own error from `main` (it converts from `io::Error`) instead of `Box<dyn Error>`.
