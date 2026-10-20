/// An RFB protocol version, as exchanged in the "RFB xxx.yyy\n" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const V3_8: Self = Self { major: 3, minor: 8 };
    /// Size of the ProtocolVersion message.
    pub const LEN: usize = 12;
}

impl ProtocolVersion {
    /// Parses "RFB xxx.yyy\n".
    pub fn parse(bytes: &[u8; Self::LEN]) -> Option<Self> {
        let digits = |b: &[u8]| -> Option<u16> {
            b.iter().try_fold(0u16, |n, &c| {
                c.is_ascii_digit().then(|| n * 10 + u16::from(c - b'0'))
            })
        };
        if &bytes[..4] != b"RFB " || bytes[7] != b'.' || bytes[11] != b'\n' {
            return None;
        }
        Some(Self {
            major: digits(&bytes[4..7])?,
            minor: digits(&bytes[8..11])?,
        })
    }

    /// Encodes as "RFB xxx.yyy\n".
    ///
    /// # Panics
    ///
    /// If `major` or `minor` is above 999.
    pub fn to_bytes(self) -> [u8; Self::LEN] {
        let text = format!("RFB {:03}.{:03}\n", self.major, self.minor);
        text.as_bytes()
            .try_into()
            .expect("major and minor fit three digits")
    }
}
