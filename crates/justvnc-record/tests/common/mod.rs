//! Recordings made by a Proxy fed real servers' bytes.
#![allow(dead_code)]

use std::time::Duration;

use justvnc_proto::client_message;
use justvnc_proto::server::SecurityType;
use justvnc_proto::{Rect, ServerInit, vnc_auth};
use justvnc_proxy::{Event as ProxyEvent, ProxyConfig, ProxySession, Tap};
use justvnc_record::{Direction, RecordingWriter};

#[macro_export]
macro_rules! client_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-client/tests/fixtures/", $name)).as_slice()
    };
}
#[macro_export]
macro_rules! server_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-server/tests/fixtures/", $name)).as_slice()
    };
}

/// A Tap that records what the Proxy forwards, one millisecond apart.
#[derive(Default)]
pub struct Recorder {
    writer: Option<RecordingWriter<Vec<u8>>>,
    clock: u64,
}

impl Recorder {
    fn write(&mut self, direction: Direction, bytes: &[u8]) {
        self.clock += 1;
        let time = Duration::from_millis(self.clock);
        let writer = self.writer.as_mut().expect("relaying started first");
        writer.write(direction, time, bytes).unwrap();
    }
}

impl Tap for Recorder {
    fn relay_started(&mut self, desktop: &ServerInit) {
        self.writer = Some(RecordingWriter::new(Vec::new(), desktop).unwrap());
    }

    fn forwarded_to_viewer(&mut self, bytes: &[u8]) {
        self.write(Direction::ToViewer, bytes);
    }

    fn forwarded_to_target(&mut self, bytes: &[u8]) {
        self.write(Direction::ToTarget, bytes);
    }
}

/// Runs a Proxy with security type None between `viewer` and `target` and returns the
/// Recording it made.
pub fn record(viewer: &[u8], target: &[u8]) -> Vec<u8> {
    let config = ProxyConfig {
        security: vec![SecurityType::None],
        ..ProxyConfig::default()
    };
    record_with(
        ProxySession::new(config).with_tap(Recorder::default()),
        viewer,
        target,
        None,
    )
}

pub fn record_with(
    mut proxy: ProxySession<Recorder>,
    viewer: &[u8],
    target: &[u8],
    password: Option<&[u8]>,
) -> Vec<u8> {
    proxy.receive_viewer(viewer).unwrap();
    // Accepting the Viewer leads to the Proxy asking for the Target.
    while let Some(event) = proxy.poll_event() {
        match event {
            ProxyEvent::Authenticate {
                challenge,
                response,
            } => {
                assert!(vnc_auth::verify(password.unwrap(), &challenge, &response));
                proxy.accept();
            }
            ProxyEvent::TargetNeeded => proxy.connect_target(None),
            _ => {}
        }
    }
    proxy.receive_target(target).unwrap();
    while proxy.poll_viewer().is_some() || proxy.poll_target().is_some() {}
    proxy.tap_mut().writer.take().unwrap().into_inner()
}

/// The handshake a scripted Viewer sends for RFB 3.8 with security type None, sharing.
pub fn viewer_handshake() -> Vec<u8> {
    [&b"RFB 003.008\n"[..], &[1], &[1]].concat()
}

pub fn full_request(width: u16, height: u16) -> [u8; 10] {
    client_message::framebuffer_update_request(
        false,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
}
