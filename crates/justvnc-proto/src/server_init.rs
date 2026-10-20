use crate::PixelFormat;

/// The desktop a server describes in ServerInit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
    pub name: DesktopName,
}

/// A desktop name as the server sent it: bytes in an unspecified, usually UTF-8, encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopName(Vec<u8>);

impl DesktopName {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The name for display, with invalid UTF-8 replaced by U+FFFD.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}
