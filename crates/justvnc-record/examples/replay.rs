//! Plays a Recording in a window, in real time, and prints what the Viewer sent.
//!
//! Usage: cargo run -p justvnc-record --features replay --example replay -- FILE
//!
//! Space pauses and resumes; Escape closes the window.

use std::time::{Duration, Instant};

use justvnc_client::Framebuffer;
use justvnc_proto::PixelFormat;
use justvnc_record::{ClientMessage, Replay, Step};
use minifb::{Key, KeyRepeat, ScaleMode, Window, WindowOptions};

/// The window's pixel for each of `fb`'s, as 0x00RRGGBB.
fn to_window(fb: &Framebuffer) -> Vec<u32> {
    let f = fb.pixel_format();
    let len = f.bytes_per_pixel();
    let scale = |value: u32, max: u16, shift: u8| {
        let c = (value >> shift) & u32::from(max);
        if max == 0 {
            0
        } else {
            c * 255 / u32::from(max)
        }
    };
    fb.pixels()
        .chunks_exact(len)
        .map(|p| {
            let value = if f.big_endian {
                p.iter().fold(0, |v, &b| v << 8 | u32::from(b))
            } else {
                p.iter().rev().fold(0, |v, &b| v << 8 | u32::from(b))
            };
            scale(value, f.red_max, f.red_shift) << 16
                | scale(value, f.green_max, f.green_shift) << 8
                | scale(value, f.blue_max, f.blue_shift)
        })
        .collect()
}

fn describe(message: &ClientMessage) -> Option<String> {
    match message {
        ClientMessage::KeyEvent { down: true, keysym } => Some(format!("key {keysym:#x}")),
        ClientMessage::PointerEvent { buttons, x, y } if *buttons != 0 => {
            Some(format!("pointer ({x}, {y}) buttons {buttons:#010b}"))
        }
        ClientMessage::CutText(text) => Some(format!("clipboard {text}")),
        ClientMessage::SetPixelFormat(format) => Some(format!("pixel format {format}")),
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: replay FILE")?;
    let file = std::io::BufReader::new(std::fs::File::open(&path)?);
    let mut replay = Replay::new(file)?;
    let desktop = replay.desktop().clone();
    println!(
        "{}x{} {:?}",
        desktop.width,
        desktop.height,
        desktop.name.to_string_lossy()
    );
    let mut fb = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    let mut window = Window::new(
        &format!("justvnc replay: {path}"),
        usize::from(desktop.width),
        usize::from(desktop.height),
        WindowOptions {
            resize: true,
            scale_mode: ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    let started = Instant::now();
    let mut paused_for = Duration::ZERO;
    let mut paused_at: Option<Instant> = None;
    let mut waiting: Option<Step> = None;
    let mut ended = false;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if window.is_key_pressed(Key::Space, KeyRepeat::No) {
            match paused_at.take() {
                Some(at) => paused_for += at.elapsed(),
                None => paused_at = Some(Instant::now()),
            }
        }
        let now = started.elapsed().saturating_sub(paused_for);
        while paused_at.is_none() && !ended {
            let step = match waiting.take() {
                Some(step) => step,
                None => match replay.next() {
                    Some(step) => step?,
                    None => {
                        println!("end of the Recording");
                        ended = true;
                        break;
                    }
                },
            };
            let time = match &step {
                Step::Shown { time, .. } | Step::Sent { time, .. } => *time,
            };
            if time > now {
                waiting = Some(step);
                break;
            }
            match step {
                Step::Shown { events, .. } => {
                    for event in &events {
                        fb.apply(event);
                    }
                }
                Step::Sent { time, message } => {
                    if let Some(text) = describe(&message) {
                        println!("{:>9.3}s Viewer sent {text}", time.as_secs_f64());
                    }
                    let format: PixelFormat = replay.pixel_format();
                    if format != fb.pixel_format() {
                        fb = Framebuffer::new(fb.width(), fb.height(), format);
                    }
                }
            }
        }
        window.update_with_buffer(
            &to_window(&fb),
            usize::from(fb.width()),
            usize::from(fb.height()),
        )?;
    }
    Ok(())
}
