use justvnc_proto::{CutText, PixelFormat, Rect};

/// The pixels a server serves, and where the Viewer's input goes.
pub trait Framebuffer {
    /// Width and height, in pixels.
    fn size(&self) -> (u16, u16);

    /// The format `read` returns pixels in. It must be a valid true-colour format.
    fn pixel_format(&self) -> PixelFormat;

    /// The pixels of `rect`, which lies inside the framebuffer: row by row from its top left,
    /// `rect.width * rect.height` pixels in `pixel_format`.
    fn read(&self, rect: Rect) -> Vec<u8>;

    /// The Viewer pressed or released `keysym` (an X11 keysym).
    fn key_event(&mut self, down: bool, keysym: u32) {
        let _ = (down, keysym);
    }

    /// The Viewer's pointer is at (`x`, `y`) with buttons 1 to 8 held in bits 0 to 7.
    fn pointer_event(&mut self, buttons: u8, x: u16, y: u16) {
        let _ = (buttons, x, y);
    }

    /// The Viewer has new clipboard text (ClientCutText).
    fn cut_text(&mut self, text: CutText) {
        let _ = text;
    }
}
