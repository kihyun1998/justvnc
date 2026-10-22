use std::time::Duration;

use justvnc_proto::{DesktopName, PixelFormat, ServerInit};
use justvnc_record::{Direction, Entry, ReadError, RecordingReader, RecordingWriter};

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

fn desktop() -> ServerInit {
    ServerInit {
        width: 640,
        height: 480,
        pixel_format: XRGB,
        name: DesktopName::new(b"desk".to_vec()),
    }
}

fn entries() -> Vec<Entry> {
    vec![
        Entry {
            direction: Direction::ToTarget,
            time: Duration::from_micros(0),
            bytes: vec![3, 0, 0, 0, 0, 0, 2, 128, 1, 224],
        },
        Entry {
            direction: Direction::ToViewer,
            time: Duration::from_millis(15),
            bytes: vec![2],
        },
        Entry {
            direction: Direction::ToViewer,
            time: Duration::from_secs(3600) + Duration::from_micros(7),
            bytes: vec![0; 300],
        },
    ]
}

fn written(entries: &[Entry]) -> Vec<u8> {
    let mut writer = RecordingWriter::new(Vec::new(), &desktop()).unwrap();
    for entry in entries {
        writer
            .write(entry.direction, entry.time, &entry.bytes)
            .unwrap();
    }
    writer.into_inner()
}

fn read_all(bytes: &[u8]) -> Result<(ServerInit, Vec<Entry>), ReadError> {
    let mut reader = RecordingReader::new(bytes)?;
    let desktop = reader.desktop().clone();
    let entries = std::iter::from_fn(|| reader.next()).collect::<Result<_, _>>()?;
    Ok((desktop, entries))
}

#[test]
fn reads_back_what_was_written() {
    let recording = written(&entries());

    assert_eq!(read_all(&recording).unwrap(), (desktop(), entries()));
}

#[test]
fn is_laid_out_as_the_map_note_says() {
    let recording = written(&entries()[1..2]);

    let init = desktop().to_bytes();
    let expected = [
        &b"JUSTVNC-RECORDING"[..],
        &1u16.to_be_bytes(),
        &(init.len() as u32).to_be_bytes(),
        &init,
        &[0],
        &15_000u64.to_be_bytes(),
        &1u32.to_be_bytes(),
        &[2],
    ]
    .concat();
    assert_eq!(recording, expected);
}

#[test]
fn refuses_what_is_not_a_recording() {
    let mut recording = written(&entries());
    recording[0] = b'X';

    assert!(matches!(
        RecordingReader::new(&recording[..]),
        Err(ReadError::NotARecording)
    ));
}

#[test]
fn refuses_a_version_it_does_not_know() {
    let mut recording = written(&entries());
    recording[18] = 2;

    assert!(matches!(
        RecordingReader::new(&recording[..]),
        Err(ReadError::UnsupportedVersion(2))
    ));
}

#[test]
fn reports_a_recording_cut_short_inside_an_entry() {
    let recording = written(&entries());

    let result = read_all(&recording[..recording.len() - 1]);

    assert!(matches!(result, Err(ReadError::Truncated)), "{result:?}");
}

#[test]
fn ends_cleanly_at_an_entry_boundary() {
    let recording = written(&entries()[..2]);

    assert_eq!(read_all(&recording).unwrap().1, entries()[..2]);
}

#[test]
fn refuses_an_unknown_direction() {
    let mut recording = written(&entries()[..1]);
    let entry = recording.len() - (1 + 8 + 4 + 10);
    recording[entry] = 7;

    assert!(matches!(
        read_all(&recording),
        Err(ReadError::Malformed("unknown direction"))
    ));
}

#[test]
fn does_not_allocate_what_an_entry_only_claims() {
    // A header claiming a 4 GiB entry, with nothing after it.
    let mut recording = written(&[]);
    recording.push(0);
    recording.extend(0u64.to_be_bytes());
    recording.extend(u32::MAX.to_be_bytes());

    let result = read_all(&recording);

    assert!(
        matches!(
            result,
            Err(ReadError::Malformed("entry is longer than any message"))
        ),
        "{result:?}"
    );
}

#[test]
fn refuses_a_desktop_name_longer_than_a_server_may_send() {
    let mut recording = written(&[]);
    // The ServerInit length, just after the magic and the version.
    recording[19..23].copy_from_slice(&u32::MAX.to_be_bytes());

    assert!(matches!(
        RecordingReader::new(&recording[..]),
        Err(ReadError::Malformed(_))
    ));
}
