//! Recordings made through a Proxy fed real servers' bytes, replayed and compared with the
//! client role decoding the same bytes directly.
#![cfg(feature = "replay")]

use std::time::Duration;

use justvnc_client::{ClientConfig, ClientSession, Event, Framebuffer};
use justvnc_proto::client_message::{self, ClientMessage};
use justvnc_proto::server::SecurityType;
use justvnc_proto::{PixelFormat, Rect, ServerInit, vnc_auth};
use justvnc_proxy::{Event as ProxyEvent, ProxyConfig, ProxySession, Tap};
use justvnc_record::{Direction, RecordingWriter, Replay, ReplayError, Step};

macro_rules! client_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-client/tests/fixtures/", $name)).as_slice()
    };
}
macro_rules! server_fixture {
    ($name:literal) => {
        include_bytes!(concat!("../../justvnc-server/tests/fixtures/", $name)).as_slice()
    };
}

/// A Tap that records what the Proxy forwards, one millisecond apart.
#[derive(Default)]
struct Recorder {
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
fn record(viewer: &[u8], target: &[u8]) -> Vec<u8> {
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

fn record_with(
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
fn viewer_handshake() -> Vec<u8> {
    [&b"RFB 003.008\n"[..], &[1], &[1]].concat()
}

fn full_request(width: u16, height: u16) -> [u8; 10] {
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

/// Replays `recording`, returning the picture and the Viewer's messages.
fn replay(recording: &[u8]) -> (Framebuffer, Vec<ClientMessage>) {
    let replay = Replay::new(recording).unwrap();
    let desktop = replay.desktop().clone();
    let mut picture = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    let mut sent = Vec::new();
    let mut last = Duration::ZERO;
    for step in replay {
        match step.unwrap() {
            Step::Shown { time, events } => {
                assert!(time >= last);
                last = time;
                for event in &events {
                    picture.apply(event);
                }
            }
            Step::Sent { time, message } => {
                assert!(time >= last);
                last = time;
                sent.push(message);
            }
        }
    }
    (picture, sent)
}

/// The picture the client role makes of `server`'s bytes on its own.
fn decoded_directly(server: &[u8]) -> Framebuffer {
    let mut session = ClientSession::new(ClientConfig::default());
    session.receive(server).unwrap();
    let events: Vec<Event> = std::iter::from_fn(|| session.poll_event()).collect();
    let Some(Event::Connected(desktop)) = events.first() else {
        panic!("{events:?}");
    };
    let mut picture = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    for event in &events {
        picture.apply(event);
    }
    picture
}

#[test]
fn replays_raw_and_copyrect_updates_into_the_same_picture() {
    let target = client_fixture!("tigervnc-3.8-none-copyrect.server.bin");
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_encodings(&[-223, -224, 1, 7, 16, 0]),
        &full_request(120, 80),
    ]
    .concat();

    let (picture, sent) = replay(&record(&viewer, target));

    assert_eq!(picture.pixels(), decoded_directly(target).pixels());
    assert!(matches!(sent[0], ClientMessage::SetEncodings(_)));
    assert!(matches!(
        sent[1],
        ClientMessage::FramebufferUpdateRequest { .. }
    ));
}

#[test]
fn replays_tight_jpeg_updates_that_end_at_last_rect() {
    // TigerVNC's updates here count 0xFFFF rectangles and end at a LastRect, which replay can
    // frame only by following the Viewer's SetEncodings, as the Proxy did.
    let target = client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin");
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_encodings(&[-223, -224, 1, 7, 16, 0, -24]),
        &full_request(256, 128),
    ]
    .concat();

    let (picture, _) = replay(&record(&viewer, target));

    assert_eq!(picture.pixels(), decoded_directly(target).pixels());
}

#[test]
fn replays_in_the_pixel_format_the_viewer_asked_for() {
    // TigerVNC answering a request for RGB565 with its ramp image.
    let target = server_fixture!("tigervnc-ramp-rgb565.server.bin");
    let rgb565 = PixelFormat {
        bits_per_pixel: 16,
        depth: 16,
        big_endian: false,
        true_colour: true,
        red_max: 31,
        green_max: 63,
        blue_max: 31,
        red_shift: 11,
        green_shift: 5,
        blue_shift: 0,
    };
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_pixel_format(&rgb565),
        &client_message::set_encodings(&[0]),
        &full_request(256, 16),
    ]
    .concat();

    let recording = record(&viewer, target);
    let mut replay = Replay::new(&recording[..]).unwrap();
    let mut pixels = None;
    for step in replay.by_ref() {
        if let Step::Shown { events, .. } = step.unwrap() {
            for event in events {
                if let Event::Rectangle(r) = event {
                    pixels = Some(r.pixels);
                }
            }
        }
    }

    assert_eq!(replay.pixel_format(), rgb565);
    let init = 18;
    let name = u32::from_be_bytes(target[init + 20..init + 24].try_into().unwrap()) as usize;
    let raw = &target[init + 24 + name + 4 + 12..];
    assert_eq!(pixels.unwrap(), raw);
}

#[test]
fn keeps_no_handshake_byte() {
    // TightVNC Viewer authenticating with "justvnc", relayed to TigerVNC.
    let viewer = server_fixture!("tightvnc-viewer-3.8-vncauth.viewer.bin");
    let handshake = server_fixture!("tightvnc-viewer-3.8-vncauth.server-handshake.bin");
    let target = client_fixture!("tigervnc-3.8-none-copyrect.server.bin");
    let challenge: [u8; 16] = handshake[14..30].try_into().unwrap();
    let response = &viewer[13..29];
    let config = ProxyConfig {
        security: vec![SecurityType::VncAuth],
        ..ProxyConfig::default()
    };
    let proxy = ProxySession::with_challenge(config, challenge).with_tap(Recorder::default());

    let recording = record_with(proxy, viewer, target, Some(b"justvnc"));

    let contains = |needle: &[u8]| recording.windows(needle.len()).any(|w| w == needle);
    assert!(!contains(&challenge), "the challenge is in the Recording");
    assert!(!contains(response), "the response is in the Recording");
    assert!(
        !contains(b"RFB 003.008"),
        "a ProtocolVersion is in the Recording"
    );
    let (_, sent) = replay(&recording);
    assert_eq!(
        sent.iter()
            .filter(|m| matches!(m, ClientMessage::KeyEvent { .. }))
            .count(),
        28,
        "the Viewer's keys are in it"
    );
}

#[test]
fn follows_the_viewers_set_encodings_for_last_rect() {
    let target = client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin");
    let init = 18;
    let name = u32::from_be_bytes(target[init + 20..init + 24].try_into().unwrap()) as usize;
    let desktop_bytes = &target[init..init + 24 + name];
    let desktop = ServerInit {
        width: 256,
        height: 128,
        pixel_format: PixelFormat::from_bytes(desktop_bytes[4..20].try_into().unwrap()),
        name: justvnc_proto::DesktopName::new(desktop_bytes[24..].to_vec()),
    };
    // An update counting 0xFFFF rectangles that ends at once with a LastRect (-224), after a
    // SetEncodings without or with LastRect.
    let update = [
        &[0, 0, 0xff, 0xff][..],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x20],
    ]
    .concat();
    let recording = |encodings: &[i32]| {
        let mut writer = RecordingWriter::new(Vec::new(), &desktop).unwrap();
        writer
            .write(
                Direction::ToTarget,
                Duration::ZERO,
                &client_message::set_encodings(encodings),
            )
            .unwrap();
        writer
            .write(Direction::ToViewer, Duration::ZERO, &update)
            .unwrap();
        writer.into_inner()
    };
    let steps = |bytes: &[u8]| {
        let mut replay = Replay::new(bytes).unwrap();
        std::iter::from_fn(|| replay.next()).collect::<Vec<_>>()
    };

    let without = steps(&recording(&[7, 0]));
    let with = steps(&recording(&[7, 0, -224]));

    assert!(
        matches!(without.last(), Some(Err(ReplayError::Message(_)))),
        "{without:?}"
    );
    assert!(with.iter().all(Result::is_ok), "{with:?}");
}

#[test]
fn refuses_an_entry_that_is_not_exactly_one_message() {
    let desktop = ServerInit {
        width: 16,
        height: 16,
        pixel_format: PixelFormat {
            bits_per_pixel: 32,
            depth: 24,
            big_endian: false,
            true_colour: true,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
        },
        name: justvnc_proto::DesktopName::new(Vec::new()),
    };
    let cases: [(Direction, Vec<u8>); 4] = [
        // Half a Raw rectangle: the update header and the rectangle header only.
        (
            Direction::ToViewer,
            [&[0, 0, 0, 1][..], &[0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0]].concat(),
        ),
        // Two Bells in one entry.
        (Direction::ToViewer, vec![2, 2]),
        // Half a KeyEvent.
        (Direction::ToTarget, vec![4, 1, 0, 0]),
        // Two PointerEvents in one entry.
        (
            Direction::ToTarget,
            [
                client_message::pointer_event(0, 1, 1),
                client_message::pointer_event(0, 2, 2),
            ]
            .concat(),
        ),
    ];
    for (direction, bytes) in cases {
        let mut writer = RecordingWriter::new(Vec::new(), &desktop).unwrap();
        writer.write(direction, Duration::ZERO, &bytes).unwrap();
        let recording = writer.into_inner();
        let mut replay = Replay::new(&recording[..]).unwrap();

        let step = replay.next();

        assert!(
            matches!(step, Some(Err(ReplayError::Message(_)))),
            "{bytes:?}: {step:?}"
        );
        assert!(replay.next().is_none(), "replay stops at an error");
    }
}
