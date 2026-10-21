#![cfg_attr(
    all(feature = "client", feature = "server", feature = "proxy", feature = "tokio"),
    doc = include_str!("../../../README.md")
)]
//! Rust VNC (RFB): client, server and Proxy roles, each behind a feature.

pub use justvnc_proto as proto;

#[cfg(feature = "client")]
pub use justvnc_client as client;

#[cfg(feature = "decode")]
pub use justvnc_decode as decode;

#[cfg(feature = "proxy")]
pub use justvnc_proxy as proxy;

#[cfg(feature = "server")]
pub use justvnc_server as server;
