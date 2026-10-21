//! Serves a moving test pattern to one Viewer at a time and prints the Viewer's input.
//!
//! Usage: cargo run -p justvnc-server --features tokio --example pattern -- [ADDRESS]
//! (default 127.0.0.1:15950). `JUSTVNC_PASSWORD` turns on VNC authentication.

use std::time::Duration;

use justvnc_server::tokio::ServeError;
use justvnc_server::{CutText, Framebuffer, PixelFormat, Rect, SecurityType, ServerConfig, verify};
use tokio::net::TcpListener;

const WIDTH: u16 = 640;
const HEIGHT: u16 = 480;
/// The moving bar's size.
const BAR: u16 = 40;

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

/// A colour gradient with a white bar that moves one step per tick.
struct Pattern {
    tick: u16,
}

impl Pattern {
    fn bar(&self) -> Rect {
        Rect {
            x: (self.tick * 8) % (WIDTH - BAR),
            y: HEIGHT / 2 - BAR / 2,
            width: BAR,
            height: BAR,
        }
    }
}

impl Framebuffer for Pattern {
    fn size(&self) -> (u16, u16) {
        (WIDTH, HEIGHT)
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        let bar = self.bar();
        let mut out = Vec::with_capacity(usize::from(rect.width) * usize::from(rect.height) * 4);
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let in_bar = (bar.x..bar.x + bar.width).contains(&x)
                    && (bar.y..bar.y + bar.height).contains(&y);
                let pixel = if in_bar {
                    [0xff, 0xff, 0xff, 0]
                } else {
                    [
                        (x + y) as u8,
                        (u32::from(y) * 255 / u32::from(HEIGHT)) as u8,
                        (u32::from(x) * 255 / u32::from(WIDTH)) as u8,
                        0,
                    ]
                };
                out.extend_from_slice(&pixel);
            }
        }
        out
    }

    fn key_event(&mut self, down: bool, keysym: u32) {
        println!("key {} {keysym:#x}", if down { "down" } else { "up" });
    }

    fn pointer_event(&mut self, buttons: u8, x: u16, y: u16) {
        if buttons != 0 {
            println!("pointer ({x}, {y}) buttons {buttons:#010b}");
        }
    }

    fn cut_text(&mut self, text: CutText) {
        println!("clipboard {text}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:15950".into());
    let password = std::env::var("JUSTVNC_PASSWORD").ok();
    let listener = TcpListener::bind(&address).await?;
    println!("listening on {address}");
    loop {
        let (stream, peer) = listener.accept().await?;
        println!("{peer} connected");
        let config = ServerConfig {
            security: vec![if password.is_some() {
                SecurityType::VncAuth
            } else {
                SecurityType::None
            }],
            ..ServerConfig::default()
        };
        let mut pattern = Pattern { tick: 0 };
        let authenticate = async |challenge: &[u8; 16], response: &[u8; 16]| {
            let password = password.as_deref().unwrap_or_default();
            verify(password.as_bytes(), challenge, response)
        };
        let mut connection =
            match justvnc_server::tokio::accept(stream, config, &mut pattern, authenticate).await {
                Ok(connection) => connection,
                Err(e) => {
                    println!("{peer} refused: {e:?}");
                    continue;
                }
            };
        let mut ticks = tokio::time::interval(Duration::from_millis(50));
        let ended = loop {
            tokio::select! {
                served = connection.serve(&mut pattern) => {
                    if let Err(e) = served {
                        break e;
                    }
                }
                _ = ticks.tick() => {
                    let before = pattern.bar();
                    pattern.tick += 1;
                    let after = pattern.bar();
                    let moved = Rect {
                        x: before.x.min(after.x),
                        width: before.x.max(after.x) + BAR - before.x.min(after.x),
                        ..before
                    };
                    if let Err(e) = connection.damage(moved, &pattern).await {
                        break e;
                    }
                }
            }
        };
        match ended {
            ServeError::Closed => println!("{peer} closed"),
            e => println!("{peer} ended: {e:?}"),
        }
    }
}
