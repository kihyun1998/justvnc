//! Reads arbitrary bytes as a Recording and replays them to the end.
#![no_main]

use justvnc_record::Replay;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(replay) = Replay::new(data) else {
        return;
    };
    let mut failed = false;
    for step in replay {
        assert!(!failed, "replay went on after an error");
        failed = step.is_err();
    }
});
