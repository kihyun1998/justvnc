//! Every public error type is a `std::error::Error` with a readable message.
#![cfg(all(
    feature = "client",
    feature = "server",
    feature = "proxy",
    feature = "decode",
    feature = "tokio"
))]

use std::error::Error;

use justvnc::client::tokio::ConnectError;
use justvnc::decode::DecodeError;
use justvnc::proto::{CutText, HandshakeError, MessageError, PixelFormat, ProtocolVersion, Rect};
use justvnc::proxy::tokio::ProxyError;
use justvnc::server::tokio::ServeError;

fn is_error<E: Error + Send + Sync + 'static>() {}

#[test]
fn every_error_type_is_a_std_error_that_can_cross_threads() {
    is_error::<HandshakeError>();
    is_error::<MessageError>();
    is_error::<justvnc::proto::NotLatin1>();
    is_error::<DecodeError>();
    is_error::<justvnc::client::Error>();
    is_error::<ConnectError>();
    is_error::<justvnc::server::Error>();
    is_error::<ServeError>();
    is_error::<justvnc::proxy::Error>();
    is_error::<ProxyError>();
}

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

#[test]
fn says_what_happened() {
    let cases: Vec<(Box<dyn Error>, &str)> = vec![
        (
            Box::new(HandshakeError::MalformedVersion),
            "the peer's ProtocolVersion message is malformed",
        ),
        (
            Box::new(HandshakeError::UnsupportedVersion {
                server: ProtocolVersion { major: 4, minor: 1 },
            }),
            "the server speaks RFB 4.1, which is not supported",
        ),
        (
            Box::new(HandshakeError::UnsupportedClientVersion {
                client: ProtocolVersion { major: 5, minor: 0 },
            }),
            "the client asked for RFB 5.0, which is not supported",
        ),
        (
            Box::new(HandshakeError::PasswordRequired),
            "the server requires VNC authentication and no password is set",
        ),
        (
            Box::new(HandshakeError::UnexpectedSecurityType { value: 9 }),
            "unexpected security type 9",
        ),
        (
            Box::new(HandshakeError::NoSupportedSecurityType {
                offered: vec![16, 19],
            }),
            "none of the security types the server offered ([16, 19]) is supported",
        ),
        (
            Box::new(HandshakeError::SecurityFailed { reason: None }),
            "authentication failed",
        ),
        (
            Box::new(HandshakeError::LimitExceeded {
                field: "name-length",
                length: 5000,
                max: 4096,
            }),
            "name-length 5000 is above the limit of 4096",
        ),
        (
            Box::new(MessageError::UnsupportedMessage { message_type: 1 }),
            "message type 1 is not supported",
        ),
        (
            Box::new(MessageError::UnsupportedEncoding { encoding: 5 }),
            "encoding 5 cannot be framed",
        ),
        (
            Box::new(MessageError::RectangleOutOfBounds {
                rect: Rect {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
            }),
            "rectangle 30x40 at (10, 20) reaches outside the framebuffer",
        ),
        (
            Box::new(MessageError::Malformed("Tight filter-id is not valid")),
            "malformed message: Tight filter-id is not valid",
        ),
        (
            Box::new(CutText::from_text("caf\u{20ac}").unwrap_err()),
            "'\u{20ac}' cannot be sent as Latin-1 clipboard text",
        ),
        (
            Box::new(DecodeError::UnsupportedEncoding { encoding: 2 }),
            "no decoder for encoding 2",
        ),
        (
            Box::new(DecodeError::WrongLength {
                expected: 16,
                actual: 12,
            }),
            "rectangle data is 12 bytes, expected 16",
        ),
        (
            Box::new(DecodeError::Corrupt("ZRLE zlib stream is invalid")),
            "corrupt rectangle data: ZRLE zlib stream is invalid",
        ),
        (
            Box::new(DecodeError::Unsupported("pixel format is not valid")),
            "not supported: pixel format is not valid",
        ),
        (
            Box::new(justvnc::client::Error::UnsupportedPixelFormat(XRGB)),
            "pixel format not supported: 32 bpp, depth 24, true colour, max 255/255/255, \
             shifts 16/8/0, little-endian",
        ),
        (
            Box::new(justvnc::server::Error::ViewerCannotResize),
            "the framebuffer changed size and the Viewer did not offer DesktopSize",
        ),
        (
            Box::new(ConnectError::Closed),
            "the server closed the connection",
        ),
        (
            Box::new(ServeError::Closed),
            "the Viewer closed the connection",
        ),
        (
            Box::new(ProxyError::ViewerClosed),
            "the Viewer closed the connection before the relay started",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn quotes_and_escapes_what_a_peer_wrote() {
    // A hostile server's reason, with a terminal escape sequence and a line break in it.
    let refused = HandshakeError::ConnectionRefused {
        reason: "no\u{1b}[31m\nway".into(),
    };
    let failed = HandshakeError::SecurityFailed {
        reason: Some("bad \"password\"".into()),
    };
    let ended = justvnc::proxy::Error::Ended("policy\u{7}".into());

    assert_eq!(
        refused.to_string(),
        r#"the server refused the connection: "no\u{1b}[31m\nway""#
    );
    assert_eq!(
        failed.to_string(),
        r#"authentication failed: "bad \"password\"""#
    );
    assert_eq!(
        ended.to_string(),
        r#"the Tap ended the session: "policy\u{7}""#
    );
}

#[test]
fn a_wrapper_that_adds_nothing_shows_the_error_it_wraps() {
    let inner = HandshakeError::PasswordRequired;
    let wrapped = ConnectError::Protocol(justvnc::client::Error::Handshake(inner.clone()));
    assert_eq!(wrapped.to_string(), inner.to_string());
    assert!(wrapped.source().is_none(), "the message is already there");

    let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
    let served = ServeError::Io(io);
    assert_eq!(served.to_string(), "reset by peer");

    let message = MessageError::UnsupportedEncoding { encoding: 5 };
    let server = justvnc::server::Error::Message(message.clone());
    assert_eq!(server.to_string(), message.to_string());
    let decode = justvnc::client::Error::Decode(DecodeError::Corrupt("x"));
    assert_eq!(decode.to_string(), "corrupt rectangle data: x");
}

#[test]
fn says_which_proxy_leg_failed_and_keeps_why_as_its_source() {
    let why = HandshakeError::SecurityFailed { reason: None };
    let cases = [
        (
            justvnc::proxy::Error::ViewerHandshake(why.clone()),
            "the Viewer's handshake failed",
        ),
        (
            justvnc::proxy::Error::TargetHandshake(why.clone()),
            "the Target's handshake failed",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert_eq!(error.source().unwrap().to_string(), why.to_string());
    }

    let unframable = MessageError::UnsupportedMessage { message_type: 9 };
    let target = ProxyError::Protocol(justvnc::proxy::Error::Target(unframable.clone()));
    assert_eq!(
        target.to_string(),
        "the Target sent a message that cannot be framed"
    );
    assert_eq!(target.source().unwrap().to_string(), unframable.to_string());
    let viewer = justvnc::proxy::Error::Viewer(unframable.clone());
    assert_eq!(
        viewer.to_string(),
        "the Viewer sent a message that cannot be framed"
    );
    assert_eq!(viewer.source().unwrap().to_string(), unframable.to_string());
}

#[test]
fn goes_into_a_boxed_error_with_the_question_mark() {
    fn connect() -> Result<(), Box<dyn Error>> {
        Err(ConnectError::Closed)?;
        Ok(())
    }

    assert_eq!(
        connect().unwrap_err().to_string(),
        "the server closed the connection"
    );
}
