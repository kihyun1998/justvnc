//! A minimal VNC viewer: shows a Target's screen in a window and forwards keyboard, mouse and
//! clipboard.
//!
//! cargo run -p justvnc-client --features tokio --example viewer -- HOST:PORT
//!
//! Set JUSTVNC_PASSWORD for VNC authentication.

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use justvnc_client::tokio::connect;
use justvnc_client::{ClientConfig, CutText, Event, Framebuffer, Password, PixelFormat, Rect};
use minifb::{InputCallback, Key, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use tokio::sync::mpsc;

/// What the window asks the connection to send.
enum Command {
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    CutText(CutText),
}

/// What the connection tells the window.
enum Update {
    Frame {
        width: usize,
        height: usize,
        pixels: Vec<u32>,
    },
    CutText(String),
    Bell,
    Closed(String),
}

/// 32 bits per pixel, little-endian, red in bits 16-23: each pixel read as a little-endian u32
/// is 0x00RRGGBB, the layout minifb draws.
const XRGB8888: PixelFormat = PixelFormat {
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

fn main() {
    let addr = std::env::args()
        .nth(1)
        .expect("usage: viewer HOST:PORT (JUSTVNC_PASSWORD for VNC authentication)");
    let password = std::env::var("JUSTVNC_PASSWORD")
        .ok()
        .map(|p| Password::from(p.as_str()));
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (update_tx, update_rx) = std_mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let reason = match runtime.block_on(run_connection(
            addr,
            password,
            command_rx,
            update_tx.clone(),
        )) {
            Ok(()) => "closed".to_string(),
            Err(e) => e,
        };
        let _ = update_tx.send(Update::Closed(reason));
    });
    run_window(command_tx, update_rx);
}

async fn run_connection(
    addr: String,
    password: Option<Password>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: std_mpsc::Sender<Update>,
) -> Result<(), String> {
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| e.to_string())?;
    let config = ClientConfig {
        password,
        pixel_format: Some(XRGB8888),
        ..ClientConfig::default()
    };
    let mut connection = connect(stream, config)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let desktop = connection.desktop().clone();
    eprintln!(
        "connected to {:?}, {}x{}",
        desktop.name.to_string_lossy(),
        desktop.width,
        desktop.height
    );
    let mut framebuffer = Framebuffer::new(desktop.width, desktop.height, XRGB8888);
    let whole = |fb: &Framebuffer| Rect {
        x: 0,
        y: 0,
        width: fb.width(),
        height: fb.height(),
    };
    connection
        .request_update(false, whole(&framebuffer))
        .await
        .map_err(|e| format!("{e:?}"))?;
    loop {
        tokio::select! {
            event = connection.next_event() => {
                let event = event.map_err(|e| format!("{e:?}"))?;
                framebuffer.apply(&event);
                match event {
                    Event::UpdateComplete => {
                        let pixels = framebuffer
                            .pixels()
                            .chunks_exact(4)
                            .map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]))
                            .collect();
                        let frame = Update::Frame {
                            width: usize::from(framebuffer.width()),
                            height: usize::from(framebuffer.height()),
                            pixels,
                        };
                        if updates.send(frame).is_err() {
                            return Ok(());
                        }
                        connection
                            .request_update(true, whole(&framebuffer))
                            .await
                            .map_err(|e| format!("{e:?}"))?;
                    }
                    Event::CutText(text) => {
                        let _ = updates.send(Update::CutText(text.to_string()));
                    }
                    Event::Bell => {
                        let _ = updates.send(Update::Bell);
                    }
                    _ => {}
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()) };
                let sent = match command {
                    Command::Key { down, keysym } => connection.key_event(down, keysym).await,
                    Command::Pointer { buttons, x, y } => connection.pointer_event(buttons, x, y).await,
                    Command::CutText(text) => connection.send_cut_text(&text).await,
                };
                sent.map_err(|e| format!("{e:?}"))?;
            }
        }
    }
}

fn run_window(commands: mpsc::UnboundedSender<Command>, updates: std_mpsc::Receiver<Update>) {
    let (mut width, mut height, mut pixels) = loop {
        match updates.recv() {
            Ok(Update::Frame {
                width,
                height,
                pixels,
            }) => break (width, height, pixels),
            Ok(Update::Closed(reason)) => return eprintln!("connection ended: {reason}"),
            Ok(_) => continue,
            Err(_) => return,
        }
    };
    let options = WindowOptions {
        resize: true,
        scale_mode: ScaleMode::AspectRatioStretch,
        ..WindowOptions::default()
    };
    let mut window = Window::new("justvnc viewer", width, height, options).unwrap();
    window.set_target_fps(60);
    window.set_input_callback(Box::new(Keyboard::new(commands.clone())));
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut last_clipboard = clipboard.as_mut().and_then(|c| c.get_text().ok());
    let mut last_clipboard_check = Instant::now();
    let mut last_pointer = None;

    while window.is_open() {
        while let Ok(update) = updates.try_recv() {
            match update {
                Update::Frame {
                    width: w,
                    height: h,
                    pixels: p,
                } => (width, height, pixels) = (w, h, p),
                Update::CutText(text) => {
                    if let Some(c) = clipboard.as_mut() {
                        let _ = c.set_text(text.clone());
                    }
                    last_clipboard = Some(text);
                }
                Update::Bell => eprintln!("bell"),
                Update::Closed(reason) => return eprintln!("connection ended: {reason}"),
            }
        }

        if let Some(pointer) = pointer_state(&window, width, height)
            && last_pointer != Some(pointer)
        {
            last_pointer = Some(pointer);
            let (buttons, x, y) = pointer;
            let _ = commands.send(Command::Pointer { buttons, x, y });
        }
        if let Some((_, scroll)) = window.get_scroll_wheel()
            && let Some((buttons, x, y)) = last_pointer
        {
            // A wheel step is a press and release of button 4 (up) or 5 (down).
            let wheel = if scroll > 0.0 { 1 << 3 } else { 1 << 4 };
            let _ = commands.send(Command::Pointer {
                buttons: buttons | wheel,
                x,
                y,
            });
            let _ = commands.send(Command::Pointer { buttons, x, y });
        }

        if last_clipboard_check.elapsed() > Duration::from_millis(500) {
            last_clipboard_check = Instant::now();
            let current = clipboard.as_mut().and_then(|c| c.get_text().ok());
            if current.is_some() && current != last_clipboard {
                last_clipboard = current.clone();
                match CutText::from_text(current.as_deref().unwrap_or_default()) {
                    Ok(text) => {
                        let _ = commands.send(Command::CutText(text));
                    }
                    Err(e) => eprintln!("clipboard not sent: {:?} is not Latin-1", e.character),
                }
            }
        }

        window.update_with_buffer(&pixels, width, height).unwrap();
    }
}

/// Buttons held (bits 0-2 for left, middle, right) and the pointer in framebuffer coordinates.
fn pointer_state(window: &Window, width: usize, height: usize) -> Option<(u8, u16, u16)> {
    let (x, y) = window.get_unscaled_mouse_pos(MouseMode::Discard)?;
    let (window_width, window_height) = window.get_size();
    // AspectRatioStretch centres the scaled framebuffer; undo the scale and the margins.
    let scale = (window_width as f32 / width as f32).min(window_height as f32 / height as f32);
    let x = (x - (window_width as f32 - width as f32 * scale) / 2.0) / scale;
    let y = (y - (window_height as f32 - height as f32 * scale) / 2.0) / scale;
    if !(0.0..width as f32).contains(&x) || !(0.0..height as f32).contains(&y) {
        return None;
    }
    let buttons = [MouseButton::Left, MouseButton::Middle, MouseButton::Right]
        .iter()
        .enumerate()
        .filter(|(_, b)| window.get_mouse_down(**b))
        .fold(0u8, |mask, (i, _)| mask | 1 << i);
    Some((buttons, x as u16, y as u16))
}

/// Turns minifb's key callbacks into KeyEvents: text through its characters, everything else
/// through X11 keysyms for the key.
struct Keyboard {
    commands: mpsc::UnboundedSender<Command>,
    /// How many Ctrl and Alt keys are held.
    ctrl_or_alt: u8,
}

impl Keyboard {
    fn new(commands: mpsc::UnboundedSender<Command>) -> Self {
        Self {
            commands,
            ctrl_or_alt: 0,
        }
    }

    fn send(&self, down: bool, keysym: u32) {
        let _ = self.commands.send(Command::Key { down, keysym });
    }
}

impl InputCallback for Keyboard {
    fn add_char(&mut self, c: u32) {
        // With Ctrl or Alt held the letter goes through set_key_state instead.
        if self.ctrl_or_alt > 0 || c < 0x20 || c == 0x7f {
            return;
        }
        // Latin-1 characters are their own keysym; the rest of Unicode is 0x01000000 + code point.
        let keysym = if c < 0x100 { c } else { 0x0100_0000 | c };
        self.send(true, keysym);
        self.send(false, keysym);
    }

    fn set_key_state(&mut self, key: Key, down: bool) {
        if matches!(
            key,
            Key::LeftCtrl | Key::RightCtrl | Key::LeftAlt | Key::RightAlt
        ) {
            self.ctrl_or_alt = if down {
                self.ctrl_or_alt + 1
            } else {
                self.ctrl_or_alt.saturating_sub(1)
            };
        }
        if let Some(keysym) = special_keysym(key) {
            self.send(down, keysym);
        } else if self.ctrl_or_alt > 0
            && let Some(keysym) = letter_or_digit_keysym(key)
        {
            self.send(down, keysym);
        }
    }
}

/// X11 keysyms (keysymdef.h) for keys that do not type text.
fn special_keysym(key: Key) -> Option<u32> {
    Some(match key {
        Key::Backspace => 0xff08,
        Key::Tab => 0xff09,
        Key::Enter | Key::NumPadEnter => 0xff0d,
        Key::Escape => 0xff1b,
        Key::Delete => 0xffff,
        Key::Insert => 0xff63,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::Left => 0xff51,
        Key::Up => 0xff52,
        Key::Right => 0xff53,
        Key::Down => 0xff54,
        Key::LeftShift => 0xffe1,
        Key::RightShift => 0xffe2,
        Key::LeftCtrl => 0xffe3,
        Key::RightCtrl => 0xffe4,
        Key::LeftAlt => 0xffe9,
        Key::RightAlt => 0xffea,
        Key::LeftSuper => 0xffeb,
        Key::RightSuper => 0xffec,
        Key::CapsLock => 0xffe5,
        Key::Menu => 0xff67,
        Key::F1 => 0xffbe,
        Key::F2 => 0xffbf,
        Key::F3 => 0xffc0,
        Key::F4 => 0xffc1,
        Key::F5 => 0xffc2,
        Key::F6 => 0xffc3,
        Key::F7 => 0xffc4,
        Key::F8 => 0xffc5,
        Key::F9 => 0xffc6,
        Key::F10 => 0xffc7,
        Key::F11 => 0xffc8,
        Key::F12 => 0xffc9,
        _ => return None,
    })
}

/// The lower-case letter or digit keysym for a key, for shortcuts such as Ctrl+C.
fn letter_or_digit_keysym(key: Key) -> Option<u32> {
    let index = key as u32;
    match index {
        0..=9 => Some(u32::from(b'0') + index),
        10..=35 => Some(u32::from(b'a') + index - 10),
        _ => None,
    }
}
