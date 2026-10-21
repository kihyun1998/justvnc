/// How a pixel's bits map to colour (PIXEL_FORMAT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// Size of PIXEL_FORMAT on the wire, padding included.
    pub const LEN: usize = 16;

    pub fn from_bytes(b: &[u8; Self::LEN]) -> Self {
        Self {
            bits_per_pixel: b[0],
            depth: b[1],
            big_endian: b[2] != 0,
            true_colour: b[3] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]),
            green_max: u16::from_be_bytes([b[6], b[7]]),
            blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10],
            green_shift: b[11],
            blue_shift: b[12],
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let [rh, rl] = self.red_max.to_be_bytes();
        let [gh, gl] = self.green_max.to_be_bytes();
        let [bh, bl] = self.blue_max.to_be_bytes();
        [
            self.bits_per_pixel,
            self.depth,
            u8::from(self.big_endian),
            u8::from(self.true_colour),
            rh,
            rl,
            gh,
            gl,
            bh,
            bl,
            self.red_shift,
            self.green_shift,
            self.blue_shift,
            0,
            0,
            0,
        ]
    }

    /// Whether the format is one rfbproto allows: 8, 16 or 32 bits per pixel, a depth no larger,
    /// and for true colour, each max one less than a power of two and each colour inside the
    /// pixel once shifted.
    pub fn is_valid(&self) -> bool {
        if !matches!(self.bits_per_pixel, 8 | 16 | 32) || self.depth > self.bits_per_pixel {
            return false;
        }
        !self.true_colour
            || [
                (self.red_max, self.red_shift),
                (self.green_max, self.green_shift),
                (self.blue_max, self.blue_shift),
            ]
            .iter()
            .all(|&(max, shift)| {
                let bits = u16::BITS - max.leading_zeros();
                u32::from(max) + 1 == 1 << bits
                    && u32::from(shift) + bits <= u32::from(self.bits_per_pixel)
            })
    }

    /// Bytes one pixel takes on the wire.
    pub fn bytes_per_pixel(&self) -> usize {
        usize::from(self.bits_per_pixel / 8)
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bpp, depth {}, ", self.bits_per_pixel, self.depth)?;
        if self.true_colour {
            write!(
                f,
                "true colour, max {}/{}/{}, shifts {}/{}/{}",
                self.red_max,
                self.green_max,
                self.blue_max,
                self.red_shift,
                self.green_shift,
                self.blue_shift
            )?;
        } else {
            f.write_str("colour-mapped")?;
        }
        let order = if self.big_endian {
            "big-endian"
        } else {
            "little-endian"
        };
        write!(f, ", {order}")
    }
}
