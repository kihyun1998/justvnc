use justvnc_proto::PixelFormat;

const XRGB: PixelFormat = PixelFormat {
    bits_per_pixel: 32,
    depth: 24,
    big_endian: false,
    true_colour: true,
    red_max: 255,
    green_max: 255,
    blue_max: 255,
    red_shift: 16,
    green_shift: 8,
    blue_shift: 0,
};

const RGB565: PixelFormat = PixelFormat {
    bits_per_pixel: 16,
    depth: 16,
    red_max: 31,
    green_max: 63,
    blue_max: 31,
    red_shift: 11,
    green_shift: 5,
    blue_shift: 0,
    ..XRGB
};

const BGR233: PixelFormat = PixelFormat {
    bits_per_pixel: 8,
    depth: 8,
    red_max: 7,
    green_max: 7,
    blue_max: 3,
    red_shift: 0,
    green_shift: 3,
    blue_shift: 6,
    ..XRGB
};

#[test]
fn accepts_the_formats_real_servers_and_viewers_use() {
    for format in [XRGB, RGB565, BGR233] {
        assert!(format.is_valid(), "{format:?}");
    }
    // Some servers send depth equal to bits-per-pixel (rfbproto).
    assert!(PixelFormat { depth: 32, ..XRGB }.is_valid());
}

#[test]
fn refuses_bits_per_pixel_other_than_8_16_or_32() {
    for bits_per_pixel in [0, 1, 4, 24, 64] {
        assert!(
            !PixelFormat {
                bits_per_pixel,
                ..XRGB
            }
            .is_valid(),
            "{bits_per_pixel}"
        );
    }
}

#[test]
fn refuses_a_depth_above_bits_per_pixel() {
    assert!(
        !PixelFormat {
            depth: 17,
            ..RGB565
        }
        .is_valid()
    );
}

#[test]
fn refuses_a_colour_that_does_not_fit_in_the_pixel() {
    // The fuzzer's crash: blue shifted 38 bits up a 32-bit pixel.
    assert!(
        !PixelFormat {
            blue_shift: 38,
            ..XRGB
        }
        .is_valid()
    );
    assert!(
        !PixelFormat {
            red_shift: 25,
            ..XRGB
        }
        .is_valid()
    );
    assert!(
        !PixelFormat {
            red_shift: 12,
            ..RGB565
        }
        .is_valid()
    );
    assert!(
        !PixelFormat {
            green_shift: 255,
            ..BGR233
        }
        .is_valid()
    );
    assert!(
        PixelFormat {
            red_shift: 24,
            ..XRGB
        }
        .is_valid()
    );
}

#[test]
fn refuses_a_max_that_is_not_one_less_than_a_power_of_two() {
    assert!(
        !PixelFormat {
            red_max: 254,
            ..XRGB
        }
        .is_valid()
    );
    assert!(
        !PixelFormat {
            green_max: 0x100,
            ..XRGB
        }
        .is_valid()
    );
}

#[test]
fn ignores_the_colour_fields_of_a_colour_mapped_format() {
    let mapped = PixelFormat {
        true_colour: false,
        red_max: 0,
        red_shift: 200,
        ..BGR233
    };

    assert!(mapped.is_valid());
}
