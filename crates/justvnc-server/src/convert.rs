use justvnc_proto::PixelFormat;

/// `pixels` in `from` rewritten in `to`. Both are valid true-colour formats.
pub(crate) fn convert(pixels: &[u8], from: &PixelFormat, to: &PixelFormat) -> Vec<u8> {
    if from == to {
        return pixels.to_vec();
    }
    let (from_len, to_len) = (from.bytes_per_pixel(), to.bytes_per_pixel());
    let channels = [
        (from.red_max, from.red_shift, to.red_max, to.red_shift),
        (
            from.green_max,
            from.green_shift,
            to.green_max,
            to.green_shift,
        ),
        (from.blue_max, from.blue_shift, to.blue_max, to.blue_shift),
    ];
    let mut out = Vec::with_capacity(pixels.len() / from_len * to_len);
    for pixel in pixels.chunks_exact(from_len) {
        let value = read(pixel, from.big_endian);
        let converted = channels
            .iter()
            .map(|&(from_max, from_shift, to_max, to_shift)| {
                let c = (value >> from_shift) & u32::from(from_max);
                scale(c, from_max, to_max) << to_shift
            })
            .fold(0, |a, b| a | b);
        write(converted, to_len, to.big_endian, &mut out);
    }
    out
}

/// `c` from 0..=`from_max` to 0..=`to_max`, rounded half up.
fn scale(c: u32, from_max: u16, to_max: u16) -> u32 {
    if from_max == 0 {
        return 0;
    }
    let (from_max, to_max) = (u32::from(from_max), u32::from(to_max));
    (c * to_max + from_max.div_ceil(2)) / from_max
}

fn read(pixel: &[u8], big_endian: bool) -> u32 {
    let fold = |v: u32, &b: &u8| v << 8 | u32::from(b);
    if big_endian {
        pixel.iter().fold(0, fold)
    } else {
        pixel.iter().rev().fold(0, fold)
    }
}

fn write(value: u32, len: usize, big_endian: bool, out: &mut Vec<u8>) {
    let bytes = value.to_le_bytes();
    if big_endian {
        out.extend(bytes[..len].iter().rev());
    } else {
        out.extend_from_slice(&bytes[..len]);
    }
}
