//! What a real Viewer sent: TightVNC Viewer 2.8.85 on Windows, connected to the `pattern`
//! example (640x480, VNC authentication with password "justvnc") through
//! `interop/record_relay.py`. The maintainer typed, clicked, scrolled and copied text.

use justvnc_proto::server_message::{Framed, ServerMessageReader};
use justvnc_server::{
    CutText, Event, Framebuffer, Limits, PixelFormat, Rect, SecurityType, ServerConfig,
    ServerSession, verify,
};

const VIEWER: &[u8] = include_bytes!("fixtures/tightvnc-viewer-3.8-vncauth.viewer.bin");
/// What the server sent up to ServerInit, which holds the challenge the Viewer answered.
const HANDSHAKE: &[u8] =
    include_bytes!("fixtures/tightvnc-viewer-3.8-vncauth.server-handshake.bin");

const XRGB: PixelFormat = PixelFormat {
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
};

#[derive(Default)]
struct Recorder {
    keys: Vec<(bool, u32)>,
    pointer: usize,
    cut_text: Vec<CutText>,
}

impl Framebuffer for Recorder {
    fn size(&self) -> (u16, u16) {
        (640, 480)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        vec![0x80; usize::from(rect.width) * usize::from(rect.height) * 4]
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        self.keys.push((down, keysym));
    }

    fn pointer_event(&mut self, _: u8, _: u16, _: u16) {
        self.pointer += 1;
    }

    fn cut_text(&mut self, text: CutText) {
        self.cut_text.push(text);
    }
}

#[test]
fn serves_what_tightvnc_viewer_sent() {
    let challenge = HANDSHAKE[14..30].try_into().unwrap();
    let mut session = ServerSession::with_challenge(
        ServerConfig {
            security: vec![SecurityType::VncAuth],
            ..ServerConfig::default()
        },
        challenge,
    );
    let mut recorder = Recorder::default();
    let mut sent = Vec::new();

    session.receive(VIEWER, &mut recorder).unwrap();
    let Some(Event::Authenticate {
        challenge,
        response,
    }) = session.poll_event()
    else {
        panic!("no authentication");
    };
    assert!(verify(b"justvnc", &challenge, &response));
    session.accept(&mut recorder).unwrap();
    while let Some(bytes) = session.poll_transmit() {
        sent.extend(bytes);
    }

    assert_eq!(
        session.poll_event(),
        Some(Event::Connected { shared: true })
    );
    assert_eq!(sent[..HANDSHAKE.len()], *HANDSHAKE);
    // Counted from the recording by a separate parse (see the map note).
    assert_eq!(recorder.keys.len(), 28);
    assert_eq!(
        recorder.keys[1..7],
        [
            (true, 0x61),
            (false, 0x61),
            (true, 0x62),
            (false, 0x62),
            (true, 0x63),
            (false, 0x63)
        ]
    );
    assert_eq!(recorder.pointer, 918);
    assert_eq!(
        recorder.cut_text,
        [CutText::from_latin1(b"\nThe \"close".to_vec())]
    );
}

/// TightVNC Viewer 2.8.85 with "8-bit colour" set, connected to the `pattern` example without
/// a password; the maintainer pressed `r` three times to resize the desktop.
const VIEWER_8BIT: &[u8] = include_bytes!("fixtures/tightvnc-viewer-3.8-none-8bit.viewer.bin");

#[test]
fn serves_tightvnc_viewer_in_8_bit_colour() {
    let mut session = ServerSession::with_challenge(ServerConfig::default(), [0; 16]);
    let mut recorder = Recorder::default();

    session.receive(VIEWER_8BIT, &mut recorder).unwrap();
    let mut sent = Vec::new();
    while let Some(bytes) = session.poll_transmit() {
        sent.extend(bytes);
    }

    // Framed as BGR233, as the Viewer asked: every rectangle one byte a pixel, to the last byte.
    let bgr233 = PixelFormat {
        bits_per_pixel: 8,
        depth: 8,
        red_max: 7,
        green_max: 7,
        blue_max: 3,
        red_shift: 0,
        green_shift: 3,
        blue_shift: 6,
        ..XRGB
    };
    let init = 12 + 2 + 4;
    let name = u32::from_be_bytes(sent[init + 20..init + 24].try_into().unwrap()) as usize;
    let mut reader = ServerMessageReader::new(&bgr233, 640, 480, Limits::default());
    let mut at = init + 24 + name;
    let (mut raw, mut copies, mut resizes) = (0, 0, 0);
    while let Some((framed, used)) = reader.next(&sent[at..]).unwrap() {
        at += used;
        match framed {
            Framed::Rectangle { rect, data, .. } => {
                assert_eq!(
                    data.len(),
                    usize::from(rect.width) * usize::from(rect.height)
                );
                raw += 1;
            }
            Framed::Copy { .. } => copies += 1,
            Framed::DesktopSize { .. } => resizes += 1,
            _ => {}
        }
    }
    assert_eq!(at, sent.len(), "bytes left that do not frame as BGR233");
    // The replay's host moves and resizes nothing: only the Viewer's requests are answered.
    assert!(raw > 0);
    assert_eq!((copies, resizes), (0, 0));
    assert_eq!(recorder.keys.len(), 10);
}
