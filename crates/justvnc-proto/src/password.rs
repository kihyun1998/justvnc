use zeroize::{Zeroize, ZeroizeOnDrop};

/// A VNC password. Its `Debug` output never shows the bytes, and its bytes are overwritten with
/// zeros when it is dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(Vec<u8>);

impl Password {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl ZeroizeOnDrop for Password {}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(..)")
    }
}

impl From<&str> for Password {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<Vec<u8>> for Password {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}
