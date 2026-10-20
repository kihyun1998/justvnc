use crate::MessageError;

/// Upper bounds on lengths a peer announces, checked before anything is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest desktop name accepted in ServerInit, in bytes.
    pub max_name_length: u32,
    /// Longest failure reason accepted from a server, in bytes.
    pub max_reason_length: u32,
    /// Widest framebuffer accepted from ServerInit or DesktopSize, in pixels.
    pub max_framebuffer_width: u32,
    /// Tallest framebuffer accepted from ServerInit or DesktopSize, in pixels.
    pub max_framebuffer_height: u32,
    /// Most rectangles accepted in one FramebufferUpdate.
    pub max_rectangles_per_update: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_name_length: 4096,
            max_reason_length: 4096,
            max_framebuffer_width: 8192,
            max_framebuffer_height: 8192,
            max_rectangles_per_update: 16384,
        }
    }
}

impl Limits {
    /// Checks a framebuffer size a server announced against the size limits.
    pub fn check_framebuffer(&self, width: u16, height: u16) -> Result<(), MessageError> {
        for (field, length, max) in [
            ("framebuffer-width", width, self.max_framebuffer_width),
            ("framebuffer-height", height, self.max_framebuffer_height),
        ] {
            if u32::from(length) > max {
                return Err(MessageError::LimitExceeded {
                    field,
                    length: u32::from(length),
                    max,
                });
            }
        }
        Ok(())
    }
}
