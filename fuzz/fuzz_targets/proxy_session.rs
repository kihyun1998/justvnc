//! Runs a Proxy session on arbitrary bytes from both legs, interleaved.
//!
//! Input: a flags byte, then chunks. Each chunk is a header byte (bit 7: from the Target,
//! otherwise from the Viewer; bits 0 to 6: its length) and that many bytes. Flags: bit 0, VNC
//! authentication instead of None on the viewer leg; bit 1, the Viewer accepted; bit 2, the
//! Target given a password; bit 3, the host refuses the Target instead of connecting.
#![no_main]

use justvnc_proto::server::SecurityType;
use justvnc_proto::{Password, ProtocolVersion};
use justvnc_proxy::{Event, ProxyConfig, ProxySession};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&flags, mut chunks)) = data.split_first() else {
        return;
    };
    let security = if flags & 1 != 0 {
        SecurityType::VncAuth
    } else {
        SecurityType::None
    };
    let mut proxy = ProxySession::with_challenge(
        ProxyConfig {
            version: ProtocolVersion::V3_8,
            security: vec![security],
            ..ProxyConfig::default()
        },
        [0x22; 16],
    );
    while let Some((&header, rest)) = chunks.split_first() {
        let len = usize::from(header & 0x7f).min(rest.len());
        let (bytes, after) = rest.split_at(len);
        chunks = after;
        let result = if header & 0x80 != 0 {
            proxy.receive_target(bytes)
        } else {
            proxy.receive_viewer(bytes)
        };
        while let Some(event) = proxy.poll_event() {
            match event {
                Event::Authenticate { .. } if flags & 2 != 0 => proxy.accept(),
                Event::Authenticate { .. } => proxy.reject("no"),
                Event::TargetNeeded if flags & 8 != 0 => proxy.refuse("unreachable"),
                Event::TargetNeeded => {
                    let password = (flags & 4 != 0).then(|| Password::new("justvnc-secret"));
                    proxy.connect_target(password);
                }
                Event::Connected { .. } => {}
            }
        }
        while proxy.poll_viewer().is_some() {}
        while proxy.poll_target().is_some() {}
        if let Err(e) = result {
            assert_eq!(
                proxy.receive_viewer(&[]),
                Err(e.clone()),
                "the error did not stick"
            );
            assert_eq!(proxy.receive_target(&[]), Err(e), "the error did not stick");
            break;
        }
    }
});
