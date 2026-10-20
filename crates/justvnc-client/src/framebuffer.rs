use justvnc_proto::PixelFormat;

use crate::Event;

/// A full-screen copy of the server's framebuffer, kept up to date from session events.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    width: u16,
    height: u16,
    format: PixelFormat,
    pixels: Vec<u8>,
}

impl Framebuffer {
    /// A black `width` x `height` framebuffer in `format`.
    pub fn new(width: u16, height: u16, format: PixelFormat) -> Self {
        let len = usize::from(width) * usize::from(height) * format.bytes_per_pixel();
        Self {
            width,
            height,
            format,
            pixels: vec![0; len],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.format
    }

    /// The pixels, row by row from the top left.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Applies a rectangle or a resize. Other events leave the framebuffer as it is, and so do
    /// the parts of a rectangle that fall outside it.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Rectangle(rectangle) => self.put(rectangle),
            Event::DesktopResized { width, height } => self.resize(*width, *height),
            _ => {}
        }
    }

    fn put(&mut self, rectangle: &crate::Rectangle) {
        let bpp = self.format.bytes_per_pixel();
        let rect = rectangle.rect;
        let x = usize::from(rect.x);
        let width = usize::from(rect.width).min(usize::from(self.width).saturating_sub(x));
        let row_len = usize::from(rect.width) * bpp;
        if row_len == 0 {
            return;
        }
        for (row, source) in rectangle.pixels.chunks_exact(row_len).enumerate() {
            let y = usize::from(rect.y) + row;
            if y >= usize::from(self.height) {
                break;
            }
            let start = (y * usize::from(self.width) + x) * bpp;
            self.pixels[start..start + width * bpp].copy_from_slice(&source[..width * bpp]);
        }
    }

    /// Resizes, keeping the top-left part both sizes share.
    fn resize(&mut self, width: u16, height: u16) {
        let mut resized = Self::new(width, height, self.format);
        let bpp = self.format.bytes_per_pixel();
        let keep = usize::from(self.width.min(width)) * bpp;
        for y in 0..usize::from(self.height.min(height)) {
            let from = y * usize::from(self.width) * bpp;
            let to = y * usize::from(width) * bpp;
            resized.pixels[to..to + keep].copy_from_slice(&self.pixels[from..from + keep]);
        }
        *self = resized;
    }
}
