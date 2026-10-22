//! What each umbrella feature turns on in the role crates.

#[cfg(all(feature = "client", feature = "jpeg"))]
#[test]
fn jpeg_reaches_the_client_role() {
    let config = justvnc::client::ClientConfig::default();

    assert_eq!(config.jpeg_quality, Some(8));
}

#[cfg(all(feature = "decode", feature = "jpeg"))]
#[test]
fn jpeg_reaches_the_decoder() {
    use justvnc::decode::{DecodeError, Decoder};
    use justvnc::proto::{PixelFormat, Rect, encoding};

    let format = PixelFormat {
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
    let rect = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    // A Tight JPEG rectangle (control 0x90) holding 4 bytes that are not a JPEG: without the
    // feature it is refused as unsupported, with it the JPEG decoder rejects it as corrupt.
    let result = Decoder::new().decode(rect, encoding::TIGHT, &[0x90, 4, 1, 2, 3, 4], &format);

    assert!(matches!(result, Err(DecodeError::Corrupt(_))), "{result:?}");
}

#[cfg(all(feature = "record", feature = "client"))]
#[test]
fn client_with_record_turns_on_replay() {
    let recording = b"not a recording".as_slice();

    assert!(justvnc::record::Replay::new(recording).is_err());
}
