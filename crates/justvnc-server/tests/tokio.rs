//! The tokio drivers of both roles over a real TCP connection.
#![cfg(feature = "tokio")]

use justvnc_client::{ClientConfig, Event as ClientEvent};
use justvnc_server::tokio::ServeError;
use justvnc_server::{
    Error, Framebuffer, HandshakeError, Password, PixelFormat, Rect, SecurityType, ServerConfig,
    verify,
};
use tokio::net::{TcpListener, TcpStream};

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

/// A 16x8 test pattern: red is x, green is y. It records the keys it is given.
#[derive(Default)]
struct Pattern {
    keys: Vec<(bool, u32)>,
}

impl Framebuffer for Pattern {
    fn size(&self) -> (u16, u16) {
        (16, 8)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        (rect.y..rect.y + rect.height)
            .flat_map(|y| (rect.x..rect.x + rect.width).map(move |x| [0, y as u8, x as u8, 0]))
            .flatten()
            .collect()
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        self.keys.push((down, keysym));
    }
}

fn config() -> ServerConfig {
    ServerConfig {
        security: vec![SecurityType::VncAuth],
        ..ServerConfig::default()
    }
}

async fn authenticate(challenge: &[u8; 16], response: &[u8; 16]) -> bool {
    verify(b"secret", challenge, response)
}

#[tokio::test]
async fn a_viewer_connects_over_tcp_sees_the_pattern_and_types() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut pattern = Pattern::default();

    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection =
            justvnc_server::tokio::accept(stream, config(), &mut pattern, authenticate)
                .await
                .unwrap();
        loop {
            match connection.serve(&mut pattern).await {
                Ok(()) => {}
                Err(ServeError::Closed) => return,
                Err(e) => panic!("{e:?}"),
            }
        }
    };
    let client = async {
        let stream = TcpStream::connect(address).await.unwrap();
        let config = ClientConfig {
            password: Some(Password::new("secret")),
            ..ClientConfig::default()
        };
        let mut connection = justvnc_client::tokio::connect(stream, config)
            .await
            .unwrap();
        let whole = Rect {
            x: 0,
            y: 0,
            width: 16,
            height: 8,
        };
        connection.request_update(false, whole).await.unwrap();
        let mut pixels = None;
        loop {
            match connection.next_event().await.unwrap() {
                ClientEvent::Rectangle(r) => pixels = Some(r),
                ClientEvent::UpdateComplete => break,
                _ => {}
            }
        }
        connection.key_event(true, 0x61).await.unwrap();
        pixels.unwrap()
    };
    let (_, rectangle) = tokio::join!(server, client);

    assert_eq!(rectangle.pixels, Pattern::default().read(rectangle.rect));
    assert_eq!(pattern.keys, [(true, 0x61)]);
}

#[tokio::test]
async fn a_viewer_with_the_wrong_password_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut pattern = Pattern::default();

    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        justvnc_server::tokio::accept(stream, config(), &mut pattern, authenticate)
            .await
            .map(|_| ())
    };
    let client = async {
        let stream = TcpStream::connect(address).await.unwrap();
        let config = ClientConfig {
            password: Some(Password::new("wrong")),
            ..ClientConfig::default()
        };
        justvnc_client::tokio::connect(stream, config)
            .await
            .map(|_| ())
    };
    let (served, connected) = tokio::join!(server, client);

    let failed = HandshakeError::SecurityFailed {
        reason: Some("Authentication failure".into()),
    };
    let Err(ServeError::Protocol(Error::Handshake(server_saw))) = served else {
        panic!("{served:?}");
    };
    assert_eq!(server_saw, failed);
    let Err(justvnc_client::tokio::ConnectError::Protocol(client_saw)) = connected else {
        panic!("{connected:?}");
    };
    assert_eq!(client_saw, justvnc_client::Error::Handshake(failed));
}

#[tokio::test]
async fn a_serve_cancelled_while_writing_loses_no_bytes() {
    // A pipe that holds 64 bytes: a 16x8 update (524 bytes) blocks while it is written.
    let (server_io, client_io) = tokio::io::duplex(64);
    let mut pattern = Pattern::default();
    let accepting = justvnc_server::tokio::accept(
        server_io,
        ServerConfig::default(),
        &mut pattern,
        authenticate,
    );
    let connecting = justvnc_client::tokio::connect(client_io, ClientConfig::default());
    let (server, client) = tokio::join!(accepting, connecting);
    let (mut server, mut client) = (server.unwrap(), client.unwrap());
    let whole = Rect {
        x: 0,
        y: 0,
        width: 16,
        height: 8,
    };
    client.request_update(false, whole).await.unwrap();

    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        server.serve(&mut pattern),
    )
    .await;
    assert!(cancelled.is_err(), "the write should still be blocked");

    let serving = async {
        loop {
            if server.serve(&mut pattern).await.is_err() {
                return;
            }
        }
    };
    let receiving = async {
        let mut pixels = None;
        loop {
            match client.next_event().await.unwrap() {
                ClientEvent::Rectangle(r) => pixels = Some(r),
                ClientEvent::UpdateComplete => return pixels.unwrap(),
                _ => {}
            }
        }
    };
    // Lost bytes leave the client waiting for the rest of the update.
    let receiving = tokio::time::timeout(std::time::Duration::from_secs(5), receiving);
    let rectangle = tokio::select! {
        r = receiving => r.expect("the whole update arrives"),
        _ = serving => panic!("the server stopped"),
    };
    assert_eq!(rectangle.rect, whole);
    assert_eq!(rectangle.pixels, Pattern::default().read(whole));
}
