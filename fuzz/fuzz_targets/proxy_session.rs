//! Runs a Proxy session on arbitrary bytes from both legs, interleaved.
//!
//! Input: a flags byte, then chunks. Each chunk is a header byte (bit 7: from the Target,
//! otherwise from the Viewer; bits 0 to 6: its length) and that many bytes. Flags: bit 0, VNC
//! authentication instead of None on the viewer leg; bit 1, the Viewer accepted; bit 2, the
//! Target given a password; bit 3, the host refuses the Target instead of connecting; bit 4, a
//! Tap that drops every third Viewer message, replaces SetEncodings, swaps the Target's Bell
//! and clipboard text, and ends the session at the 200th update piece.
#![no_main]

use justvnc_proto::server::SecurityType;
use justvnc_proto::{Password, ProtocolVersion};
use justvnc_proxy::{
    ClientMessage, CutText, Event, Framed, ProxyConfig, ProxySession, Tap, TargetMessage,
    UpdateVerdict, Verdict,
};
use libfuzzer_sys::fuzz_target;

/// A Tap whose verdicts change from message to message.
struct Meddler {
    on: bool,
    count: usize,
}

impl Tap for Meddler {
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        self.count += 1;
        match message {
            _ if !self.on => Verdict::Forward,
            ClientMessage::SetEncodings(_) => {
                Verdict::Replace(ClientMessage::SetEncodings(vec![7, 5, -224, 16, 0, -239]))
            }
            _ if self.count.is_multiple_of(3) => Verdict::Drop,
            _ => Verdict::Forward,
        }
    }

    fn target_message(&mut self, message: &TargetMessage) -> Verdict<TargetMessage> {
        match message {
            _ if !self.on => Verdict::Forward,
            TargetMessage::Bell => Verdict::Replace(TargetMessage::CutText(CutText::from_latin1(
                b"bell".to_vec(),
            ))),
            TargetMessage::CutText(_) => Verdict::Replace(TargetMessage::Bell),
        }
    }

    fn target_update(&mut self, _: &Framed) -> UpdateVerdict {
        self.count += 1;
        if self.on && self.count == 200 {
            UpdateVerdict::End("enough".into())
        } else {
            UpdateVerdict::Forward
        }
    }
}

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
    )
    .with_tap(Meddler {
        on: flags & 16 != 0,
        count: 0,
    });
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
                Event::ViewerAuthenticated
                | Event::TargetConnected { .. }
                | Event::Closed { .. } => {}
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
