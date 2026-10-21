//! What a real Viewer sent: TightVNC Viewer 2.8.85 on Windows, connected to the `pattern`
//! example (640x480, VNC authentication with password "justvnc") through
//! `interop/record_relay.py`. The maintainer typed, clicked, scrolled and copied text.

use justvnc_server::{
    CutText, Event, Framebuffer, PixelFormat, Rect, SecurityType, ServerConfig, ServerSession,
    verify,
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
        vec![0; usize::from(rect.width) * usize::from(rect.height) * 4]
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
