//! Recordings exported as keyframes and changed regions, rebuilt and compared with replay.
#![cfg(feature = "frames")]

mod common;

use std::time::Duration;

use common::{full_request, record, viewer_handshake};
use justvnc_client::{Event, Framebuffer};
use justvnc_proto::client_message;
use justvnc_proto::{DesktopName, PixelFormat, Rect, ServerInit, encoding, server_message};
use justvnc_record::{
    Direction, Frame, FrameKind, Frames, FramesConfig, RecordingWriter, Replay, Step, export_frames,
};

/// The picture's red, green and blue, 8 bits each, written apart from the exporter's own
/// conversion: each colour scaled to 0..=255, rounded half up.
fn rgb(fb: &Framebuffer) -> Vec<u8> {
    let f = fb.pixel_format();
    let len = usize::from(f.bits_per_pixel / 8);
    let mut out = Vec::new();
    for p in fb.pixels().chunks(len) {
        let mut v: u32 = 0;
        for i in 0..len {
            let byte = if f.big_endian { p[i] } else { p[len - 1 - i] };
            v = (v << 8) | u32::from(byte);
        }
        for (max, shift) in [
            (f.red_max, f.red_shift),
            (f.green_max, f.green_shift),
            (f.blue_max, f.blue_shift),
        ] {
            let max = u32::from(max);
            let c = (v >> shift) & max;
            let half_up = max.div_ceil(2);
            out.push((c * 255 + half_up).checked_div(max).unwrap_or(0) as u8);
        }
    }
    out
}

/// The replayed picture, as RGB, after every completed update, with its time.
fn replayed_pictures(recording: &[u8]) -> Vec<(Duration, u16, u16, Vec<u8>)> {
    let mut replay = Replay::new(recording).unwrap();
    let desktop = replay.desktop().clone();
    let mut fb = Framebuffer::new(desktop.width, desktop.height, desktop.pixel_format);
    let mut out = Vec::new();
    while let Some(step) = replay.next() {
        match step.unwrap() {
            Step::Shown { time, events } => {
                for event in &events {
                    fb.apply(event);
                    if *event == Event::UpdateComplete {
                        out.push((time, fb.width(), fb.height(), rgb(&fb)));
                    }
                }
            }
            Step::Sent { .. } => {
                if replay.pixel_format() != fb.pixel_format() {
                    fb = Framebuffer::new(fb.width(), fb.height(), replay.pixel_format());
                }
            }
        }
    }
    out
}

/// A screen rebuilt from frames: a keyframe replaces it, a region is drawn onto it.
#[derive(Default)]
struct Rebuilt {
    width: u16,
    height: u16,
    rgb: Vec<u8>,
}

impl Rebuilt {
    fn apply(&mut self, frame: &Frame) {
        assert_eq!(
            frame.rgb.len(),
            usize::from(frame.rect.width) * usize::from(frame.rect.height) * 3
        );
        if frame.kind == FrameKind::Keyframe {
            assert_eq!((frame.rect.x, frame.rect.y), (0, 0));
            self.width = frame.rect.width;
            self.height = frame.rect.height;
            self.rgb = frame.rgb.clone();
            return;
        }
        assert!(!self.rgb.is_empty(), "a region before any keyframe");
        let r = frame.rect;
        assert!(
            r.x + r.width <= self.width && r.y + r.height <= self.height,
            "{r:?}"
        );
        for row in 0..usize::from(r.height) {
            let from = row * usize::from(r.width) * 3;
            let to = ((usize::from(r.y) + row) * usize::from(self.width) + usize::from(r.x)) * 3;
            let n = usize::from(r.width) * 3;
            self.rgb[to..to + n].copy_from_slice(&frame.rgb[from..from + n]);
        }
    }
}

/// Checks that the frames, applied in order, give the replayed picture at every completed
/// update.
fn assert_rebuilds(recording: &[u8], frames: &[Frame]) {
    let pictures = replayed_pictures(recording);
    assert!(!pictures.is_empty());
    let mut rebuilt = Rebuilt::default();
    let mut next = 0;
    for (time, width, height, rgb) in pictures {
        while next < frames.len() && frames[next].time <= time {
            rebuilt.apply(&frames[next]);
            next += 1;
        }
        assert_eq!(
            (rebuilt.width, rebuilt.height),
            (width, height),
            "at {time:?}"
        );
        assert!(
            rebuilt.rgb == rgb,
            "the rebuilt picture differs at {time:?}"
        );
    }
    assert_eq!(next, frames.len(), "frames after the last update");
}

fn frames(recording: &[u8], config: FramesConfig) -> Vec<Frame> {
    Frames::new(Replay::new(recording).unwrap(), config)
        .collect::<Result<_, _>>()
        .unwrap()
}

fn tigervnc_copyrect() -> Vec<u8> {
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_encodings(&[-223, -224, 1, 7, 16, 0]),
        &full_request(120, 80),
        &full_request(120, 80),
        &full_request(120, 80),
    ]
    .concat();
    record(
        &viewer,
        client_fixture!("tigervnc-3.8-none-copyrect.server.bin"),
    )
}

#[test]
fn rebuilds_tigervncs_raw_and_copyrect_session() {
    let recording = tigervnc_copyrect();

    let frames = frames(&recording, FramesConfig::default());

    assert_rebuilds(&recording, &frames);
    assert_eq!(frames[0].kind, FrameKind::Keyframe);
    assert!(frames[1..].iter().all(|f| f.kind == FrameKind::Region));
    // The incremental update moved the logo: four copies and two Raw rectangles.
    assert!(frames.len() > 6, "{}", frames.len());
}

#[test]
fn rebuilds_tigervncs_tight_jpeg_session() {
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_encodings(&[-223, -224, 1, 7, 16, 0, -24]),
        &full_request(256, 128),
        &full_request(256, 128),
    ]
    .concat();
    let recording = record(
        &viewer,
        client_fixture!("tigervnc-3.8-none-tight-jpeg.server.bin"),
    );

    assert_rebuilds(&recording, &frames(&recording, FramesConfig::default()));
}

#[test]
fn rebuilds_a_session_in_rgb565_and_starts_it_with_a_keyframe() {
    let rgb565 = PixelFormat {
        bits_per_pixel: 16,
        depth: 16,
        big_endian: false,
        true_colour: true,
        red_max: 31,
        green_max: 63,
        blue_max: 31,
        red_shift: 11,
        green_shift: 5,
        blue_shift: 0,
    };
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_pixel_format(&rgb565),
        &client_message::set_encodings(&[0]),
        &full_request(256, 16),
    ]
    .concat();
    let recording = record(&viewer, server_fixture!("tigervnc-ramp-rgb565.server.bin"));

    let frames = frames(&recording, FramesConfig::default());

    assert_rebuilds(&recording, &frames);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].kind, FrameKind::Keyframe);
    // Column 255 of the red ramp is 31 in RGB565: full red.
    assert_eq!(frames[0].rgb[255 * 3..255 * 3 + 3], [255, 0, 0]);
}

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

/// A Recording of a 16x16 desktop written entry by entry.
struct Script(RecordingWriter<Vec<u8>>);

impl Script {
    fn new() -> Self {
        let desktop = ServerInit {
            width: 16,
            height: 16,
            pixel_format: XRGB,
            name: DesktopName::new(Vec::new()),
        };
        Self(RecordingWriter::new(Vec::new(), &desktop).unwrap())
    }

    /// One update of one Raw rectangle of `colour`, at `secs`.
    fn update(&mut self, secs: u64, rect: Rect, colour: [u8; 4]) {
        let pixels = colour.repeat(usize::from(rect.width) * usize::from(rect.height));
        let bytes = [
            &server_message::framebuffer_update(1)[..],
            &server_message::rectangle(rect, encoding::RAW),
            &pixels,
        ]
        .concat();
        self.shown(secs, &bytes);
    }

    fn shown(&mut self, secs: u64, bytes: &[u8]) {
        self.0
            .write(Direction::ToViewer, Duration::from_secs(secs), bytes)
            .unwrap();
    }

    fn sent(&mut self, secs: u64, bytes: &[u8]) {
        self.0
            .write(Direction::ToTarget, Duration::from_secs(secs), bytes)
            .unwrap();
    }

    fn finish(self) -> Vec<u8> {
        self.0.into_inner()
    }
}

fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn kinds(frames: &[Frame]) -> Vec<(u64, FrameKind)> {
    frames.iter().map(|f| (f.time.as_secs(), f.kind)).collect()
}

#[test]
fn writes_nothing_while_the_screen_stands_still() {
    let mut script = Script::new();
    script.update(0, rect(0, 0, 16, 16), [1, 2, 3, 0]);
    for secs in 1..300 {
        script.sent(secs, &client_message::pointer_event(0, 3, 3));
    }
    // An update that changes nothing, long after the keyframe interval.
    script.shown(300, &server_message::framebuffer_update(0));
    let recording = script.finish();

    let frames = frames(&recording, FramesConfig::default());

    assert_eq!(kinds(&frames), [(0, FrameKind::Keyframe)]);
    assert_rebuilds(&recording, &frames);
}

#[test]
fn writes_a_keyframe_at_the_first_change_after_the_interval() {
    let mut script = Script::new();
    script.update(0, rect(0, 0, 16, 16), [1, 2, 3, 0]);
    script.update(10, rect(2, 2, 3, 3), [9, 9, 9, 0]);
    script.update(59, rect(4, 4, 1, 1), [8, 8, 8, 0]);
    script.update(100, rect(5, 5, 2, 2), [7, 7, 7, 0]);
    script.update(110, rect(6, 6, 2, 2), [6, 6, 6, 0]);
    let recording = script.finish();
    let config = FramesConfig {
        keyframe_interval: Duration::from_secs(60),
    };

    let frames = frames(&recording, config);

    assert_eq!(
        kinds(&frames),
        [
            (0, FrameKind::Keyframe),
            (10, FrameKind::Region),
            (59, FrameKind::Region),
            (100, FrameKind::Keyframe),
            (110, FrameKind::Region),
        ]
    );
    assert_eq!(frames[1].rect, rect(2, 2, 3, 3));
    assert_rebuilds(&recording, &frames);
}

#[test]
fn writes_a_keyframe_after_a_resize_and_after_a_pixel_format_change() {
    let mut script = Script::new();
    script.update(0, rect(0, 0, 16, 16), [1, 2, 3, 0]);
    let resize = [
        &server_message::framebuffer_update(1)[..],
        &server_message::rectangle(rect(0, 0, 20, 10), encoding::DESKTOP_SIZE),
    ]
    .concat();
    script.shown(1, &resize);
    script.update(2, rect(0, 0, 20, 10), [4, 5, 6, 0]);
    let bgr233 = PixelFormat {
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
    script.sent(3, &client_message::set_pixel_format(&bgr233));
    let pixels = [0b1100_0111u8; 20 * 10];
    let update = [
        &server_message::framebuffer_update(1)[..],
        &server_message::rectangle(rect(0, 0, 20, 10), encoding::RAW),
        &pixels,
    ]
    .concat();
    script.shown(4, &update);
    let recording = script.finish();

    let frames = frames(&recording, FramesConfig::default());

    assert_eq!(
        kinds(&frames),
        [
            (0, FrameKind::Keyframe),
            (1, FrameKind::Keyframe),
            (2, FrameKind::Region),
            (4, FrameKind::Keyframe),
        ]
    );
    assert_eq!((frames[1].rect.width, frames[1].rect.height), (20, 10));
    // Red 7 of 7 and blue 3 of 3: magenta.
    assert_eq!(frames[3].rgb[..3], [255, 0, 255]);
    assert_rebuilds(&recording, &frames);
}

/// The index written by `export_frames`, read back, and the PNGs it names, decoded.
fn read_export(dir: &std::path::Path) -> Vec<Frame> {
    let index = std::fs::read_to_string(dir.join("index.csv")).unwrap();
    let mut lines = index.lines();
    assert_eq!(lines.next(), Some("time_us,kind,x,y,width,height,file"));
    lines
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            assert_eq!(f.len(), 7, "{line}");
            let num = |i: usize| f[i].parse::<u16>().unwrap();
            let kind = match f[1] {
                "keyframe" => FrameKind::Keyframe,
                "region" => FrameKind::Region,
                other => panic!("kind {other}"),
            };
            let decoder = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(dir.join(f[6])).unwrap(),
            ));
            let mut reader = decoder.read_info().unwrap();
            let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut rgb).unwrap();
            assert_eq!(info.color_type, png::ColorType::Rgb);
            assert_eq!(
                (info.width, info.height),
                (u32::from(num(4)), u32::from(num(5)))
            );
            rgb.truncate(info.buffer_size());
            Frame {
                time: Duration::from_micros(f[0].parse().unwrap()),
                kind,
                rect: rect(num(2), num(3), num(4), num(5)),
                rgb,
            }
        })
        .collect()
}

#[test]
fn exports_pngs_and_an_index_that_rebuild_the_session() {
    let recording = tigervnc_copyrect();
    let dir = std::env::temp_dir().join(format!("justvnc-frames-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let written = export_frames(
        Frames::new(
            Replay::new(&recording[..]).unwrap(),
            FramesConfig::default(),
        ),
        &dir,
    )
    .unwrap();

    let read = read_export(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(written, read.len());
    assert_eq!(read, frames(&recording, FramesConfig::default()));
    assert_rebuilds(&recording, &read);
}

#[test]
fn rebuilds_a_session_in_big_endian_32_bit() {
    let be32 = PixelFormat {
        big_endian: true,
        red_shift: 0,
        green_shift: 8,
        blue_shift: 16,
        ..XRGB
    };
    let viewer = [
        &viewer_handshake()[..],
        &client_message::set_pixel_format(&be32),
        &client_message::set_encodings(&[0]),
        &full_request(256, 16),
    ]
    .concat();
    let recording = record(&viewer, server_fixture!("tigervnc-ramp-be32.server.bin"));

    let frames = frames(&recording, FramesConfig::default());

    assert_rebuilds(&recording, &frames);
    // Rows 4-7 hold the green ramp: column 200 is green 200.
    let at = (4 * 256 + 200) * 3;
    assert_eq!(frames[0].rgb[at..at + 3], [0, 200, 0]);
}

#[test]
fn takes_what_an_update_changed_when_the_recording_ends_inside_it() {
    let mut script = Script::new();
    script.update(0, rect(0, 0, 16, 16), [1, 2, 3, 0]);
    // An update of two rectangles, cut off after the first.
    let first = [
        &server_message::framebuffer_update(2)[..],
        &server_message::rectangle(rect(1, 1, 2, 2), encoding::RAW),
        &[5u8, 5, 5, 0].repeat(4),
    ]
    .concat();
    script.shown(7, &first);
    let recording = script.finish();

    let frames = frames(&recording, FramesConfig::default());

    assert_eq!(
        kinds(&frames),
        [(0, FrameKind::Keyframe), (7, FrameKind::Region)]
    );
    assert_eq!(frames[1].rect, rect(1, 1, 2, 2));
    assert_eq!(frames[1].rgb[..3], [5, 5, 5]);
}
