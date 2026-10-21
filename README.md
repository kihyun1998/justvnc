# justvnc

A Rust implementation of VNC (the RFB protocol, RFC 6143 and the community rfbproto). It plays the client role, the server role, or both at once as a Proxy between a Viewer and a Target.

The protocol core is sans-IO: each role is a state machine that takes bytes in and gives bytes and events out, and never owns a socket or a runtime. The `tokio` feature adds thin drivers over Tokio streams.

## Features

Nothing is on by default. Turn on the roles you use:

| Feature | What it adds |
|---|---|
| `client` | `justvnc::client`: connect to a server, receive decoded rectangles, send input |
| `server` | `justvnc::server`: serve a framebuffer you supply through a trait, receive input |
| `proxy` | `justvnc::proxy`: relay a Viewer to a Target, with a Tap that sees every message |
| `decode` | `justvnc::decode`: pixel decoding on its own (the client role includes it) |
| `tokio` | the Tokio drivers of whichever roles are on |
| `jpeg` | Tight JPEG for the client role and the decoder, in pure Rust |

`justvnc::proto`, the handshakes, messages and framing, is always there.

Supported: RFB 3.3, 3.7 and 3.8; security types None and VNC authentication; Raw, CopyRect, ZRLE and Tight (JPEG with `jpeg`), with DesktopSize and LastRect.

## Client

```toml
justvnc = { version = "0.0.0", features = ["client", "tokio"] }
```

```rust,no_run
use justvnc::client::tokio::connect;
use justvnc::client::{ClientConfig, Event, Rect};
use tokio::net::TcpStream;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stream = TcpStream::connect("127.0.0.1:5900").await?;
    let mut connection = connect(stream, ClientConfig::default()).await?;
    let desktop = connection.desktop();
    let whole = Rect { x: 0, y: 0, width: desktop.width, height: desktop.height };
    connection.request_update(false, whole).await?;
    loop {
        match connection.next_event().await? {
            Event::Rectangle(r) => println!("{:?}: {} bytes", r.rect, r.pixels.len()),
            Event::UpdateComplete => return Ok(()),
            _ => {}
        }
    }
}
```

`ClientConfig` takes a password for VNC authentication and a pixel format to ask for. `justvnc::client::Framebuffer` applies the events to a whole-screen buffer.

## Server

```toml
justvnc = { version = "0.0.0", features = ["server", "tokio"] }
```

```rust,no_run
use justvnc::server::tokio::accept;
use justvnc::server::{Framebuffer, PixelFormat, Rect, ServerConfig};
use tokio::net::TcpListener;

/// A grey 320x240 screen that prints the keys it is given.
struct Grey;

impl Framebuffer for Grey {
    fn size(&self) -> (u16, u16) {
        (320, 240)
    }

    fn pixel_format(&self) -> PixelFormat {
        PixelFormat {
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
        }
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        vec![0x80; usize::from(rect.width) * usize::from(rect.height) * 4]
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        println!("key {keysym:#x} down: {down}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:5900").await?;
    let (stream, _) = listener.accept().await?;
    let mut screen = Grey;
    // With SecurityType::VncAuth in the config, decide with justvnc::server::verify.
    let mut connection =
        accept(stream, ServerConfig::default(), &mut screen, |_, _| async { true }).await?;
    loop {
        connection.serve(&mut screen).await?;
    }
}
```

Report changed pixels with `Connection::damage`, moved ones with `copy`, and a new size with `resize`; `serve` and these can share a `tokio::select!`.

## Proxy

```toml
justvnc = { version = "0.0.0", features = ["proxy", "tokio"] }
```

```rust,no_run
use justvnc::proxy::tokio::serve;
use justvnc::proxy::{
    ClientMessage, Password, ProxyConfig, SecurityType, Tap, TargetMessage, Verdict, verify,
};
use tokio::net::{TcpListener, TcpStream};

/// Drops clipboard text both ways.
struct NoClipboard;

impl Tap for NoClipboard {
    fn viewer_message(&mut self, message: &ClientMessage) -> Verdict<ClientMessage> {
        match message {
            ClientMessage::CutText(_) => Verdict::Drop,
            _ => Verdict::Forward,
        }
    }

    fn target_message(&mut self, message: &TargetMessage) -> Verdict<TargetMessage> {
        match message {
            TargetMessage::CutText(_) => Verdict::Drop,
            _ => Verdict::Forward,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:5901").await?;
    let (viewer, _) = listener.accept().await?;
    let config = ProxyConfig {
        security: vec![SecurityType::VncAuth],
        ..ProxyConfig::default()
    };
    // The Viewer authenticates with a one-time token; only then is the Target reached, with
    // its real password, which the Viewer never sees.
    let authenticate = |challenge, response| async move {
        verify(b"one-time", &challenge, &response)
    };
    let connect = || async {
        let target = TcpStream::connect("127.0.0.1:5900")
            .await
            .map_err(|e| format!("Target unreachable: {e}"))?;
        Ok((target, Some(Password::new("target-secret"))))
    };
    let ended = serve(viewer, config, NoClipboard, authenticate, connect).await?;
    println!("{ended:?}");
    Ok(())
}
```

The Proxy frames every message in both directions without decoding pixels, and narrows the Viewer's encodings to the ones it can frame.

## Development

Gates, interop tests and fixture recording: [`docs/development.md`](docs/development.md).

## License

MIT or Apache-2.0, at your option.
