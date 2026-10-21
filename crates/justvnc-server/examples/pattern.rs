//! Serves a moving test pattern to one Viewer at a time and prints the Viewer's input.
//!
//! Usage: cargo run -p justvnc-server --features tokio --example pattern -- [ADDRESS]
//! (default 127.0.0.1:15950). `JUSTVNC_PASSWORD` turns on VNC authentication.
//!
//! The white square moves by CopyRect where the Viewer offers it. The `r` key switches the
//! desktop between 640x480 and 800x600.

use std::time::Duration;

use justvnc_server::tokio::ServeError;
use justvnc_server::{CutText, Framebuffer, PixelFormat, Rect, SecurityType, ServerConfig, verify};
use tokio::net::TcpListener;

const SIZES: [(u16, u16); 2] = [(640, 480), (800, 600)];
/// The moving square's size, and how far it moves each tick.
const BAR: u16 = 40;
const STEP: u16 = 8;

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

/// A colour gradient with a white square that moves one step per tick.
struct Pattern {
    size: usize,
    tick: u32,
    resize_asked: bool,
}

impl Pattern {
    fn width(&self) -> u16 {
        SIZES[self.size].0
    }

    fn height(&self) -> u16 {
        SIZES[self.size].1
    }

    fn bar(&self) -> Rect {
        let travel = u32::from((self.width() - BAR) / STEP);
        Rect {
            x: (self.tick % travel) as u16 * STEP,
            y: self.height() / 2 - BAR / 2,
            width: BAR,
            height: BAR,
        }
    }
}

impl Framebuffer for Pattern {
    fn size(&self) -> (u16, u16) {
        (self.width(), self.height())
    }

    fn pixel_format(&self) -> PixelFormat {
        XRGB
    }

    fn read(&self, rect: Rect) -> Vec<u8> {
        let bar = self.bar();
        let (width, height) = (u32::from(self.width()), u32::from(self.height()));
        let mut out = Vec::with_capacity(usize::from(rect.width) * usize::from(rect.height) * 4);
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let in_bar = (bar.x..bar.x + bar.width).contains(&x)
                    && (bar.y..bar.y + bar.height).contains(&y);
                let pixel = if in_bar {
                    [0xff, 0xff, 0xff, 0]
                } else {
                    let (x, y) = (u32::from(x), u32::from(y));
                    [
                        (x + y) as u8,
                        (y * 255 / height) as u8,
                        (x * 255 / width) as u8,
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
        if down && keysym == u32::from(b'r') {
            self.resize_asked = true;
        }
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
        let mut pattern = Pattern {
            size: 0,
            tick: 0,
            resize_asked: false,
        };
        let authenticate = |challenge, response| {
            let password = password.clone().unwrap_or_default();
            async move { verify(password.as_bytes(), &challenge, &response) }
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
                    if std::mem::take(&mut pattern.resize_asked) {
                        pattern.size = 1 - pattern.size;
                        println!("resized to {:?}", pattern.size());
                        if let Err(e) = connection.resize(&pattern).await {
                            break e;
                        }
                    }
                }
                _ = ticks.tick() => {
                    let before = pattern.bar();
                    pattern.tick += 1;
                    let after = pattern.bar();
                    let sent = if after.x > before.x {
                        // The square moved right: copy it, then redraw the strip it left.
                        let strip = Rect { width: after.x - before.x, ..before };
                        match connection.copy(after, before.x, before.y, &pattern).await {
                            Ok(()) => connection.damage(strip, &pattern).await,
                            e => e,
                        }
                    } else {
                        // It wrapped round to the left edge.
                        match connection.damage(before, &pattern).await {
                            Ok(()) => connection.damage(after, &pattern).await,
                            e => e,
                        }
                    };
                    if let Err(e) = sent {
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
