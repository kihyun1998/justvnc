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
