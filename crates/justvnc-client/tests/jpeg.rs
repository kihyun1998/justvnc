//! Tight JPEG, with the `jpeg` feature on.
#![cfg(feature = "jpeg")]

mod common;

use justvnc_client::{ClientConfig, ClientSession, Event, Rect, Rectangle};

/// TigerVNC (256x128) answering two full updates of `make_zrle_image.py`'s image in Tight, with
/// JPEG quality level 8 (-24) offered: each update holds two JPEG rectangles and a fill.
const SERVER: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-tight-jpeg.server.bin");
const CLIENT: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-tight-jpeg.client.bin");
/// The same JPEG rectangles decoded by Pillow's libjpeg (`interop/tight_jpeg_reference.py`):
/// x, y, width, height as U16, then red, green, blue bytes.
const LIBJPEG: &[u8] = include_bytes!("fixtures/tigervnc-3.8-none-tight-jpeg.libjpeg.bin");

struct Reference {
    rect: Rect,
    rgb: Vec<u8>,
}

fn libjpeg_rectangles() -> Vec<Reference> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < LIBJPEG.len() {
        let field = |i: usize| u16::from_be_bytes([LIBJPEG[at + i], LIBJPEG[at + i + 1]]);
        let rect = Rect {
            x: field(0),
            y: field(2),
            width: field(4),
            height: field(6),
        };
        let len = usize::from(rect.width) * usize::from(rect.height) * 3;
        out.push(Reference {
            rect,
            rgb: LIBJPEG[at + 8..at + 8 + len].to_vec(),
        });
        at += 8 + len;
    }
    out
}

/// Red, green and blue of each pixel in TigerVNC's format: 32 bpp little-endian 0x00RRGGBB.
fn rgb(pixels: &[u8]) -> Vec<u8> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect()
}

#[test]
fn decodes_tigervncs_jpeg_as_libjpeg_does() {
    let mut session = ClientSession::new(ClientConfig::default());

    session.receive(SERVER).unwrap();
    let whole = Rect {
        x: 0,
        y: 0,
        width: 256,
        height: 128,
    };
    session.request_update(false, whole);
    session.request_update(false, whole);

    let transmit: Vec<u8> = std::iter::from_fn(|| session.poll_transmit())
        .flatten()
        .collect();
    assert_eq!(
        common::without_set_encodings(&transmit, 14),
        common::without_set_encodings(CLIENT, 14)
    );
    let references = libjpeg_rectangles();
    let decoded: Vec<Rectangle> = std::iter::from_fn(|| session.poll_event())
        .filter_map(|e| match e {
            Event::Rectangle(r) if references.iter().any(|j| j.rect == r.rect) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(decoded.len(), references.len());
    for (ours, theirs) in decoded.iter().zip(&references) {
        assert_eq!(ours.rect, theirs.rect);
        let ours = rgb(&ours.pixels);
        let diffs: Vec<u8> = ours
            .iter()
            .zip(&theirs.rgb)
            .map(|(a, b)| a.abs_diff(*b))
            .collect();
        let max = *diffs.iter().max().unwrap();
        let mean = diffs.iter().map(|&d| f64::from(d)).sum::<f64>() / diffs.len() as f64;
        // Measured against libjpeg: max 2, mean 0.02 (IDCT and upsampling rounding). The JPEG
        // loss itself, against the Raw picture, is mean 1.5 to 2.6 and max 28.
        assert!(
            max <= 3 && mean <= 0.1,
            "{:?}: max {max}, mean {mean:.3} from libjpeg",
            theirs.rect
        );
    }
}

#[test]
fn asks_for_jpeg_quality_8_by_default_and_none_when_unset() {
    let set_encodings = |config: ClientConfig| {
        let mut session = ClientSession::new(config);
        // Up to the end of ServerInit; SetEncodings follows the 14 handshake bytes it sends.
        let start = 12 + 1 + usize::from(SERVER[12]) + 4;
        let name = u32::from_be_bytes(SERVER[start + 20..start + 24].try_into().unwrap()) as usize;
        session.receive(&SERVER[..start + 24 + name]).unwrap();
        let transmit: Vec<u8> = std::iter::from_fn(|| session.poll_transmit())
            .flatten()
            .collect();
        transmit[14 + 4..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|e| i32::from_be_bytes(*e))
            .collect::<Vec<i32>>()
    };

    let default = set_encodings(ClientConfig::default());
    let unset = set_encodings(ClientConfig {
        jpeg_quality: None,
        ..ClientConfig::default()
    });

    // rfbproto: quality levels run from -32 (0) to -23 (9).
    assert_eq!(default.last(), Some(&-24));
    assert!(!unset.iter().any(|e| (-32..=-23).contains(e)), "{unset:?}");
}
