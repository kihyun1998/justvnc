//! Connects to a real VNC server. Run with `--features tokio -- --ignored` and
//! JUSTVNC_TIGERVNC_ADDR pointing at the container in `interop/tigervnc`.
#![cfg(feature = "tokio")]

use justvnc_client::ClientConfig;
use justvnc_client::tokio::connect;

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
