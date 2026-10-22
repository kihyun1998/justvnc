//! Exports a Recording as PNG keyframes and changed regions, with an index.
//!
//! Usage: cargo run -p justvnc-record --features frames --example frames -- FILE DIR [SECONDS]
//!
//! SECONDS is the keyframe interval (default 60).

use std::time::Duration;

use justvnc_record::{Frames, FramesConfig, Replay, export_frames};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(file), Some(dir)) = (args.next(), args.next()) else {
        return Err("usage: frames FILE DIR [SECONDS]".into());
    };
    let mut config = FramesConfig::default();
    if let Some(secs) = args.next() {
        config.keyframe_interval = Duration::from_secs(secs.parse()?);
    }
    let replay = Replay::new(std::io::BufReader::new(std::fs::File::open(&file)?))?;
    let written = export_frames(Frames::new(replay, config), dir.as_ref())?;
    println!("{written} images and index.csv in {dir}");
    Ok(())
}
