mod common;

use justvnc_client::{ClientConfig, ClientSession, Event, Framebuffer, Rect};

/// x11vnc (256x128) showing `interop/make_zrle_image.py`'s image, answering two full updates in
/// ZRLE over one connection: every subencoding appears, and the second update continues the first
/// one's zlib stream.
const ZRLE_SERVER: &[u8] = include_bytes!("fixtures/x11vnc-3.8-none-zrle.server.bin");
const ZRLE_CLIENT: &[u8] = include_bytes!("fixtures/x11vnc-3.8-none-zrle.client.bin");
/// The same screen over a second connection that asked for Raw only.
const REFERENCE: &[u8] = include_bytes!("fixtures/x11vnc-3.8-none-zrle-reference-raw.server.bin");

const WHOLE: Rect = Rect {
    x: 0,
    y: 0,
    width: 256,
    height: 128,
};

/// Feeds a recording and returns the framebuffer after each FramebufferUpdate.
fn frames(server: &[u8], session: &mut ClientSession) -> Vec<Vec<u8>> {
    session.receive(server).unwrap();
    let mut events = std::iter::from_fn(|| session.poll_event());
    let Some(Event::Connected(desktop)) = events.next() else {
        panic!("expected Connected first");
    };
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    let mut frames = Vec::new();
    for event in events {
        framebuffer.apply(&event);
        if event == Event::UpdateComplete {
            frames.push(framebuffer.pixels().to_vec());
        }
    }
    frames
}

#[test]
fn decodes_x11vncs_zrle_as_its_raw_picture_across_one_zlib_stream() {
    let mut reference_session = ClientSession::new(ClientConfig::default());
    let [reference] = frames(REFERENCE, &mut reference_session)
        .try_into()
        .unwrap();
    let mut session = ClientSession::new(ClientConfig::default());

    let frames = frames(ZRLE_SERVER, &mut session);
    session.request_update(false, WHOLE);
    session.request_update(false, WHOLE);

    let transmit: Vec<u8> = std::iter::from_fn(|| session.poll_transmit())
        .flatten()
        .collect();
    // Version, security type and ClientInit take 14 bytes; SetEncodings follows.
    assert_eq!(
        common::without_set_encodings(&transmit, 14),
        common::without_set_encodings(ZRLE_CLIENT, 14)
    );
    assert_eq!(frames.len(), 2);
    // x11vnc's Raw pixels carry non-zero bytes outside the colour bits, which ZRLE's 3-byte
    // CPIXELs drop, so only the colour bits are compared.
    for (i, frame) in frames.iter().enumerate() {
        assert!(
            colour_bits(frame) == colour_bits(&reference),
            "update {i} differs from the Raw picture"
        );
    }
}

/// Each pixel's red, green and blue bits in x11vnc's format: 32 bpp little-endian, 0x00RRGGBB.
fn colour_bits(frame: &[u8]) -> Vec<u32> {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_le_bytes(*p) & 0x00ff_ffff)
        .collect()
}

#[test]
fn rejects_zrle_data_longer_than_the_limit_before_it_arrives() {
    let config = ClientConfig {
        limits: justvnc_client::Limits {
            max_compressed_length: 100,
            ..justvnc_client::Limits::default()
        },
        ..ClientConfig::default()
    };
    let mut session = ClientSession::new(config);
    // Up to the end of ServerInit: version, security, SecurityResult, then 24 bytes and the name.
    let name_length = u32::from_be_bytes(ZRLE_SERVER[38..42].try_into().unwrap()) as usize;
    session.receive(&ZRLE_SERVER[..42 + name_length]).unwrap();

    // One ZRLE rectangle (encoding 16) announcing 101 bytes of zlib data; none of it follows.
    let update = [
        0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 8, 0, 0, 0, 16, 0, 0, 0, 101,
    ];
    let result = session.receive(&update);

    assert_eq!(
        result,
        Err(justvnc_client::Error::Message(
            justvnc_client::MessageError::LimitExceeded {
                field: "zrle-length",
                length: 101,
                max: 100
            }
        ))
    );
}
