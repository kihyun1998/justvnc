/// Clipboard text as RFB carries it: ISO 8859-1 (Latin-1), with lines ended by LF alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutText(Vec<u8>);

/// Text holding a character Latin-1 cannot represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotLatin1 {
    /// The first character that does not fit.
    pub character: char,
}

impl CutText {
    /// Text that is already Latin-1, as it arrives from a peer.
    pub fn from_latin1(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Encodes `text` as Latin-1, turning CRLF into LF.
    pub fn from_text(text: &str) -> Result<Self, NotLatin1> {
        let mut bytes = Vec::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' && chars.peek() == Some(&'\n') {
                continue;
            }
            let byte = u8::try_from(u32::from(c)).map_err(|_| NotLatin1 { character: c })?;
            bytes.push(byte);
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for CutText {
    /// Every Latin-1 byte is the Unicode code point of the same value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0
            .iter()
            .try_for_each(|&b| std::fmt::Write::write_char(f, char::from(b)))
    }
}
