//! justvnc's client role, through the Proxy, to justvnc's server role, all over TCP.
#![cfg(feature = "tokio")]

use justvnc_client::{ClientConfig, Event as ClientEvent};
use justvnc_proxy::tokio::{Ended, ProxyError, serve};
use justvnc_proxy::{
    ClientMessage, Error, ForwardAll, HandshakeError, Password, PixelFormat, ProxyConfig,
    SecurityType, Tap, Verdict, verify,
};
use justvnc_server::{Framebuffer, Rect, ServerConfig};
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

/// A 16x8 pattern behind the Proxy, recording the keys that reach it.
#[derive(Default)]
struct Pattern {
    keys: Vec<(bool, u32)>,
    cut_text: Vec<justvnc_server::CutText>,
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
            .flat_map(|y| (rect.x..rect.x + rect.width).map(move |x| [7, y as u8, x as u8, 0]))
            .flatten()
            .collect()
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        self.keys.push((down, keysym));
    }

    fn cut_text(&mut self, text: justvnc_server::CutText) {
        self.cut_text.push(text);
    }
}

const WHOLE: Rect = Rect {
    x: 0,
    y: 0,
    width: 16,
    height: 8,
};

/// The Target: justvnc's server with its own password, "target-secret".
async fn target(listener: TcpListener, pattern: &mut Pattern) {
    let (stream, _) = listener.accept().await.unwrap();
    let config = ServerConfig {
        security: vec![SecurityType::VncAuth],
        ..ServerConfig::default()
    };
    let authenticate =
        |challenge, response| async move { verify(b"target-secret", &challenge, &response) };
    let mut connection = justvnc_server::tokio::accept(stream, config, pattern, authenticate)
        .await
        .unwrap();
    while connection.serve(pattern).await.is_ok() {}
}

/// The Proxy: the Viewer authenticates with the one-time token "token123", and the Proxy logs
/// in to the Target with the Target's real password.
async fn proxy(listener: TcpListener, target: std::net::SocketAddr) -> Result<Ended, ProxyError> {
    proxy_with(listener, target, ForwardAll).await
}

async fn proxy_with(
    listener: TcpListener,
    target: std::net::SocketAddr,
    tap: impl Tap,
) -> Result<Ended, ProxyError> {
    let (stream, _) = listener.accept().await.unwrap();
    let config = ProxyConfig {
        security: vec![SecurityType::VncAuth],
        ..ProxyConfig::default()
    };
    let authenticate =
        |challenge, response| async move { verify(b"token123", &challenge, &response) };
    let connect = || async move {
        let stream = TcpStream::connect(target)
            .await
            .map_err(|e| e.to_string())?;
        Ok((stream, Some(Password::new("target-secret"))))
    };
    serve(stream, config, tap, authenticate, connect).await
}

#[tokio::test]
async fn a_viewer_reaches_the_target_through_the_proxy() {
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target_listener.local_addr().unwrap();
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy_listener.local_addr().unwrap();
    let mut pattern = Pattern::default();

    let viewer = async {
        let stream = TcpStream::connect(proxy_address).await.unwrap();
        let config = ClientConfig {
            password: Some(Password::new("token123")),
            ..ClientConfig::default()
        };
        let mut connection = justvnc_client::tokio::connect(stream, config)
            .await
            .unwrap();
        connection.request_update(false, WHOLE).await.unwrap();
        let mut pixels = None;
        loop {
            match connection.next_event().await.unwrap() {
                ClientEvent::Rectangle(r) => pixels = Some(r),
                ClientEvent::UpdateComplete => break,
                _ => {}
            }
        }
        connection.key_event(true, 0x61).await.unwrap();
        // Closing the Viewer ends the relay.
        drop(connection);
        pixels.unwrap()
    };
    // A Proxy that loses bytes leaves the Viewer waiting.
    let (_, proxied, rectangle) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            target(target_listener, &mut pattern),
            proxy(proxy_listener, target_address),
            viewer
        )
    })
    .await
    .expect("the session completes");

    assert_eq!(rectangle.pixels, Pattern::default().read(WHOLE));
    assert_eq!(pattern.keys, [(true, 0x61)]);
    assert_eq!(proxied.unwrap(), Ended::ViewerClosed);
}

#[tokio::test]
async fn a_viewer_is_told_when_the_target_cannot_be_reached() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy_listener.local_addr().unwrap();

    let proxy = async {
        let (stream, _) = proxy_listener.accept().await.unwrap();
        let connect =
            || async { Err::<(TcpStream, Option<Password>), _>("Target unreachable".into()) };
        serve(
            stream,
            ProxyConfig::default(),
            ForwardAll,
            |_, _| async { true },
            connect,
        )
        .await
    };
    let viewer = async {
        let stream = TcpStream::connect(proxy_address).await.unwrap();
        justvnc_client::tokio::connect(stream, ClientConfig::default())
            .await
            .map(|_| ())
    };
    let (proxied, connected) = tokio::join!(proxy, viewer);

    let refused = HandshakeError::SecurityFailed {
        reason: Some("Target unreachable".into()),
    };
    let Err(ProxyError::Protocol(Error::ViewerHandshake(proxy_saw))) = proxied else {
        panic!("{proxied:?}");
    };
    assert_eq!(proxy_saw, refused);
    let Err(justvnc_client::tokio::ConnectError::Protocol(viewer_saw)) = connected else {
        panic!("{connected:?}");
    };
    assert_eq!(viewer_saw, justvnc_client::Error::Handshake(refused));
}

/// Compiles only if `serve`'s future can go to `tokio::spawn` with closures that capture an
/// owned password, as a host serving many Viewers writes them.
#[allow(dead_code)]
fn serve_can_be_spawned(viewer: TcpStream, token: String, target_password: String) {
    let authenticate = move |challenge, response| {
        let token = token.clone();
        async move { verify(token.as_bytes(), &challenge, &response) }
    };
    let connect = move || async move {
        let stream = TcpStream::connect("127.0.0.1:1")
            .await
            .map_err(|e| e.to_string())?;
        Ok((stream, Some(Password::new(target_password))))
    };
    tokio::spawn(serve(
        viewer,
        ProxyConfig::default(),
        ForwardAll,
        authenticate,
        connect,
    ));
}

/// A Tap that blocks clipboard text from the Viewer, ends the session at the second key, and
/// keeps the names of the events it is told.
#[derive(Default)]
struct Guard {
    keys: usize,
    events: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl Tap for Guard {
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        match message {
            ClientMessage::CutText(_) => Verdict::Drop,
            ClientMessage::KeyEvent { .. } => {
                self.keys += 1;
                if self.keys == 2 {
                    Verdict::End("one key only".into())
                } else {
                    Verdict::Forward
                }
            }
            _ => Verdict::Forward,
        }
    }

    fn event(&mut self, event: &justvnc_proxy::Event) {
        let name = format!("{event:?}");
        self.events
            .borrow_mut()
            .push(name.split([' ', '{', '(']).next().unwrap().to_owned());
    }
}

#[tokio::test]
async fn the_tap_filters_and_ends_a_session_served_over_tcp() {
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_address = target_listener.local_addr().unwrap();
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy_listener.local_addr().unwrap();
    let mut pattern = Pattern::default();
    let guard = Guard::default();
    let events = guard.events.clone();

    let viewer = async {
        let stream = TcpStream::connect(proxy_address).await.unwrap();
        let config = ClientConfig {
            password: Some(Password::new("token123")),
            ..ClientConfig::default()
        };
        let mut connection = justvnc_client::tokio::connect(stream, config)
            .await
            .unwrap();
        let text = justvnc_client::CutText::from_text("secret").unwrap();
        connection.send_cut_text(&text).await.unwrap();
        connection.key_event(true, 0x61).await.unwrap();
        connection.key_event(false, 0x61).await.unwrap();
        // The Proxy ends the session and drops the Viewer's connection.
        while connection.next_event().await.is_ok() {}
    };
    let (_, proxied, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            target(target_listener, &mut pattern),
            proxy_with(proxy_listener, target_address, guard),
            viewer
        )
    })
    .await
    .expect("the session completes");

    assert_eq!(proxied.unwrap(), Ended::ByTap("one key only".into()));
    assert_eq!(
        pattern.keys,
        [(true, 0x61)],
        "the second key was never relayed"
    );
    assert_eq!(pattern.cut_text, [], "the clipboard text was dropped");
    assert_eq!(
        *events.borrow(),
        [
            "Authenticate",
            "ViewerAuthenticated",
            "TargetNeeded",
            "TargetConnected",
            "Closed"
        ]
    );
}
