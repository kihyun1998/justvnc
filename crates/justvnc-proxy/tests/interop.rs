//! justvnc's client role through the Proxy to a real TigerVNC. Run like the client role's
//! interop tests (`docs/development.md`).
#![cfg(feature = "tokio")]

use std::time::Duration;

use justvnc_client::{ClientConfig, CutText, Event, Framebuffer, Rect};
use justvnc_proxy::tokio::serve;
use justvnc_proxy::{Password, ProxyConfig, SecurityType, verify};
use tokio::net::{TcpListener, TcpStream};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

/// Starts a Proxy for one Viewer in front of the Target at `target`, which it logs in to with
/// `password`. The Viewer authenticates with "token123". Returns the Proxy's address.
async fn proxy_to(target: String, password: Option<&'static str>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
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
            Ok((stream, password.map(Password::new)))
        };
        let _ = serve(stream, config, authenticate, connect).await;
    });
    address
}

async fn viewer(proxy: std::net::SocketAddr) -> justvnc_client::tokio::Connection<TcpStream> {
    let stream = TcpStream::connect(proxy).await.unwrap();
    let config = ClientConfig {
        password: Some(Password::new("token123")),
        ..ClientConfig::default()
    };
    justvnc_client::tokio::connect(stream, config)
        .await
        .unwrap()
}

/// Xvnc's `-retro` root window: a 4-pixel diagonal stipple, white where this holds.
fn retro_is_white(x: usize, y: usize) -> bool {
    x % 4 == [3, 1, 2, 0][y % 4]
}

#[tokio::test]
#[ignore = "needs a running TigerVNC started with XVNC_ARGS=-retro; set JUSTVNC_TIGERVNC_ADDR"]
async fn captures_a_frame_from_tigervnc_through_the_proxy() {
    let proxy = proxy_to(env("JUSTVNC_TIGERVNC_ADDR"), None).await;

    let frame = tokio::time::timeout(Duration::from_secs(20), async {
        let mut connection = viewer(proxy).await;
        let desktop = connection.desktop().clone();
        let area = Rect {
            x: 0,
            y: 0,
            width: desktop.width,
            height: desktop.height,
        };
        connection.request_update(false, area).await.unwrap();
        let mut framebuffer =
            Framebuffer::new(desktop.width, desktop.height, connection.pixel_format());
        loop {
            let event = connection.next_event().await.unwrap();
            framebuffer.apply(&event);
            if event == Event::UpdateComplete {
                return framebuffer;
            }
        }
    })
    .await
    .expect("no complete update in time");

    // Rows 0..300 lie above the centred cursor; every pixel there follows the stipple.
    let width = usize::from(frame.width());
    for y in 0..300 {
        for x in 0..width {
            let pixel = &frame.pixels()[(y * width + x) * 4..][..4];
            let white = pixel[..3] == [0xff, 0xff, 0xff];
            assert_eq!(white, retro_is_white(x, y), "pixel ({x}, {y}) = {pixel:?}");
        }
    }
}

#[tokio::test]
#[ignore = "needs TigerVNC with a password; set JUSTVNC_TIGERVNC_AUTH_ADDR"]
async fn logs_in_to_tigervnc_with_its_own_password_for_a_viewer_with_a_token() {
    let proxy = proxy_to(env("JUSTVNC_TIGERVNC_AUTH_ADDR"), Some("justvnc-secret")).await;

    let connection = tokio::time::timeout(Duration::from_secs(20), viewer(proxy))
        .await
        .expect("no handshake in time");

    assert!(connection.desktop().width > 0);
}

#[tokio::test]
#[ignore = "needs TigerVNC; set JUSTVNC_TIGERVNC_ADDR and JUSTVNC_TIGERVNC_EXEC"]
async fn relays_clipboard_text_to_tigervnc() {
    let proxy = proxy_to(env("JUSTVNC_TIGERVNC_ADDR"), None).await;
    let mut connection = viewer(proxy).await;

    let text = CutText::from_text("through the proxy").unwrap();
    connection.send_cut_text(&text).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let exec = env("JUSTVNC_TIGERVNC_EXEC");
    let mut words = exec.split_whitespace();
    let output = std::process::Command::new(words.next().unwrap())
        .args(words)
        .args(["sh", "-c", "DISPLAY=:0 xsel -o -b"])
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"through the proxy");
}
