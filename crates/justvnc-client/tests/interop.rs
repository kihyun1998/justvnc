//! Connects to a real VNC server. Run with `--features tokio -- --ignored` and
//! JUSTVNC_TIGERVNC_ADDR pointing at the container in `interop/tigervnc`.
#![cfg(feature = "tokio")]

use justvnc_client::tokio::connect;
use justvnc_client::{ClientConfig, Event, Framebuffer, Rect};

#[tokio::test]
#[ignore = "needs a running TigerVNC; set JUSTVNC_TIGERVNC_ADDR"]
async fn connects_to_tigervnc_and_reports_its_desktop() {
    let addr = std::env::var("JUSTVNC_TIGERVNC_ADDR").expect("JUSTVNC_TIGERVNC_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let connection = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        connect(stream, ClientConfig::default()),
    )
    .await
    .expect("handshake timed out")
    .unwrap();

    let desktop = connection.desktop();
    assert_eq!((desktop.width, desktop.height), (1024, 768));
    assert_eq!(desktop.name.to_string_lossy(), "justvnc-test");
    assert_eq!(desktop.pixel_format.bits_per_pixel, 32);
}

/// The password the `interop/tigervnc` container is started with (VNC_PASSWORD) in CI.
const TIGERVNC_PASSWORD: &str = "justvnc-secret";

#[tokio::test]
#[ignore = "needs a running TigerVNC with VNC_PASSWORD; set JUSTVNC_TIGERVNC_AUTH_ADDR"]
async fn authenticates_to_tigervnc_with_a_password() {
    let addr =
        std::env::var("JUSTVNC_TIGERVNC_AUTH_ADDR").expect("JUSTVNC_TIGERVNC_AUTH_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let config = ClientConfig {
        password: Some(TIGERVNC_PASSWORD.into()),
        ..ClientConfig::default()
    };

    let connection =
        tokio::time::timeout(std::time::Duration::from_secs(10), connect(stream, config))
            .await
            .expect("handshake timed out")
            .unwrap();

    assert_eq!(connection.desktop().name.to_string_lossy(), "justvnc-test");
}

/// Xvnc's `-retro` root window: a 4-pixel diagonal stipple, white where this holds.
fn retro_is_white(x: usize, y: usize) -> bool {
    x % 4 == [3, 1, 2, 0][y % 4]
}

#[tokio::test]
#[ignore = "needs a running TigerVNC started with XVNC_ARGS=-retro; set JUSTVNC_TIGERVNC_ADDR"]
async fn captures_a_frame_from_tigervnc() {
    let addr = std::env::var("JUSTVNC_TIGERVNC_ADDR").expect("JUSTVNC_TIGERVNC_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let frame = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut connection = connect(stream, ClientConfig::default()).await.unwrap();
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
    let (width, bpp) = (usize::from(frame.width()), 4);
    for y in 0..300 {
        for x in 0..width {
            let pixel = &frame.pixels()[(y * width + x) * bpp..][..bpp];
            let white = pixel[..3] == [0xff, 0xff, 0xff];
            assert_eq!(white, retro_is_white(x, y), "pixel ({x}, {y}) = {pixel:?}");
        }
    }
}
