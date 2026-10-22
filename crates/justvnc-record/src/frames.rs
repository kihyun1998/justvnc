//! A Recording as images: keyframes of the whole screen, and the regions each update changed.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use justvnc_client::{Event, Framebuffer};
use justvnc_proto::{PixelFormat, Rect};

use crate::{Replay, ReplayError, Step};

/// When keyframes are taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramesConfig {
    /// After this much recorded time since the last keyframe, the next change is taken as a
    /// keyframe instead of regions.
    pub keyframe_interval: Duration,
}

impl Default for FramesConfig {
    fn default() -> Self {
        Self {
            keyframe_interval: Duration::from_secs(60),
        }
    }
}

/// Whether a frame is the whole screen or one changed part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// The whole screen, at (0, 0): it replaces what came before.
    Keyframe,
    /// The part of the screen in `rect`, drawn over the screen so far.
    Region,
}

/// One image: the screen, or a part of it, as it was at `time`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Since the session started, as recorded.
    pub time: Duration,
    pub kind: FrameKind,
    pub rect: Rect,
    /// Red, green and blue, 8 bits each, row by row from the top left of `rect`.
    pub rgb: Vec<u8>,
}

/// The frames of a Recording, in order. A frame is taken at the end of each update that changed
/// something: a keyframe first, after a desktop size or pixel format change, and at the first
/// change once the keyframe interval has passed; otherwise one region per changed rectangle,
/// taken from the screen as the update left it. While the screen does not change, nothing is
/// taken.
#[derive(Debug)]
pub struct Frames<R> {
    replay: Replay<R>,
    config: FramesConfig,
    screen: Framebuffer,
    changed: Vec<Rect>,
    keyframe_due: bool,
    last_keyframe: Duration,
    last_time: Duration,
    ready: VecDeque<Frame>,
    done: bool,
}

impl<R: Read> Frames<R> {
    pub fn new(replay: Replay<R>, config: FramesConfig) -> Self {
        let desktop = replay.desktop();
        let screen = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
        Self {
            replay,
            config,
            screen,
            changed: Vec::new(),
            keyframe_due: true,
            last_keyframe: Duration::ZERO,
            last_time: Duration::ZERO,
            ready: VecDeque::new(),
            done: false,
        }
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Shown { time, events } => {
                self.last_time = time;
                for event in &events {
                    self.screen.apply(event);
                    match event {
                        Event::Rectangle(r) => self.changed.push(r.rect),
                        Event::Copy { rect, .. } => self.changed.push(*rect),
                        Event::DesktopResized { .. } => {
                            self.keyframe_due = true;
                            self.changed.clear();
                        }
                        Event::UpdateComplete => self.take(time),
                        _ => {}
                    }
                }
            }
            Step::Sent { time, .. } => {
                self.last_time = time;
                let format = self.replay.pixel_format();
                if format != self.screen.pixel_format() {
                    self.screen =
                        Framebuffer::new(self.screen.width(), self.screen.height(), format);
                    self.keyframe_due = true;
                    self.changed.clear();
                }
            }
        }
    }

    /// Takes the frames for the screen as it is at `time`.
    fn take(&mut self, time: Duration) {
        let interval_passed =
            time.saturating_sub(self.last_keyframe) >= self.config.keyframe_interval;
        if self.keyframe_due || (interval_passed && !self.changed.is_empty()) {
            let whole = Rect {
                x: 0,
                y: 0,
                width: self.screen.width(),
                height: self.screen.height(),
            };
            self.ready
                .push_back(self.frame(time, FrameKind::Keyframe, whole));
            self.keyframe_due = false;
            self.last_keyframe = time;
        } else {
            for rect in std::mem::take(&mut self.changed) {
                if let Some(rect) = clip(rect, self.screen.width(), self.screen.height()) {
                    self.ready
                        .push_back(self.frame(time, FrameKind::Region, rect));
                }
            }
        }
        self.changed.clear();
    }

    fn frame(&self, time: Duration, kind: FrameKind, rect: Rect) -> Frame {
        let format = self.screen.pixel_format();
        let len = format.bytes_per_pixel();
        let stride = usize::from(self.screen.width()) * len;
        let mut rgb = Vec::with_capacity(usize::from(rect.width) * usize::from(rect.height) * 3);
        for y in rect.y..rect.y + rect.height {
            let start = usize::from(y) * stride + usize::from(rect.x) * len;
            let row = &self.screen.pixels()[start..start + usize::from(rect.width) * len];
            for pixel in row.chunks_exact(len) {
                rgb.extend(to_rgb(pixel, &format));
            }
        }
        Frame {
            time,
            kind,
            rect,
            rgb,
        }
    }
}

impl<R: Read> Iterator for Frames<R> {
    type Item = Result<Frame, ReplayError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                return Some(Ok(frame));
            }
            if self.done {
                return None;
            }
            match self.replay.next() {
                Some(Ok(step)) => self.step(step),
                Some(Err(e)) => {
                    self.done = true;
                    return Some(Err(e));
                }
                None => {
                    // A Recording that ends inside an update: what it changed so far.
                    self.done = true;
                    if !self.changed.is_empty() {
                        self.take(self.last_time);
                    }
                }
            }
        }
    }
}

/// The part of `rect` inside a `width` x `height` screen.
fn clip(rect: Rect, width: u16, height: u16) -> Option<Rect> {
    let x2 = (u32::from(rect.x) + u32::from(rect.width)).min(u32::from(width));
    let y2 = (u32::from(rect.y) + u32::from(rect.height)).min(u32::from(height));
    (u32::from(rect.x) < x2 && u32::from(rect.y) < y2).then(|| Rect {
        x: rect.x,
        y: rect.y,
        width: (x2 - u32::from(rect.x)) as u16,
        height: (y2 - u32::from(rect.y)) as u16,
    })
}

/// One pixel in `format` as red, green and blue, each scaled to 0..=255 and rounded half up.
fn to_rgb(pixel: &[u8], format: &PixelFormat) -> [u8; 3] {
    let fold = |v: u32, &b: &u8| (v << 8) | u32::from(b);
    let value = if format.big_endian {
        pixel.iter().fold(0, fold)
    } else {
        pixel.iter().rev().fold(0, fold)
    };
    let scale = |max: u16, shift: u8| {
        let max = u32::from(max);
        if max == 0 {
            return 0;
        }
        let c = (value >> shift) & max;
        ((c * 255 + max.div_ceil(2)) / max) as u8
    };
    [
        scale(format.red_max, format.red_shift),
        scale(format.green_max, format.green_shift),
        scale(format.blue_max, format.blue_shift),
    ]
}

/// Why frames could not be exported.
#[derive(Debug)]
pub enum ExportError {
    Replay(ReplayError),
    Io(io::Error),
    Png(png::EncodingError),
}

/// Writes `frames` into `dir` (created if missing) as numbered PNG files and `index.csv`, one
/// line per image: `time_us,kind,x,y,width,height,file`, in order. Returns how many images were
/// written.
pub fn export_frames<R: Read>(frames: Frames<R>, dir: &Path) -> Result<usize, ExportError> {
    std::fs::create_dir_all(dir).map_err(ExportError::Io)?;
    let mut index =
        io::BufWriter::new(std::fs::File::create(dir.join("index.csv")).map_err(ExportError::Io)?);
    writeln!(index, "time_us,kind,x,y,width,height,file").map_err(ExportError::Io)?;
    let mut written = 0;
    for frame in frames {
        let frame = frame.map_err(ExportError::Replay)?;
        written += 1;
        let name = format!("{written:06}.png");
        let file = std::fs::File::create(dir.join(&name)).map_err(ExportError::Io)?;
        let mut encoder = png::Encoder::new(
            io::BufWriter::new(file),
            u32::from(frame.rect.width),
            u32::from(frame.rect.height),
        );
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(ExportError::Png)?;
        writer
            .write_image_data(&frame.rgb)
            .map_err(ExportError::Png)?;
        writer.finish().map_err(ExportError::Png)?;
        let kind = match frame.kind {
            FrameKind::Keyframe => "keyframe",
            FrameKind::Region => "region",
        };
        let r = frame.rect;
        writeln!(
            index,
            "{},{kind},{},{},{},{},{name}",
            frame.time.as_micros(),
            r.x,
            r.y,
            r.width,
            r.height
        )
        .map_err(ExportError::Io)?;
    }
    index.flush().map_err(ExportError::Io)?;
    Ok(written)
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Replay(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::Png(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Replay(e) => e.source(),
            Self::Io(e) => e.source(),
            Self::Png(e) => e.source(),
        }
    }
}
