//! For the test that reads a real far machine and must change nothing
//! there (`tests/far_mini.rs`): a switch that keeps every visit from
//! beginning, since a visit subscribes to a session, for a snapshot as
//! for an answer; and a list of every call written to a gateway, by its
//! method alone, so the test can say exactly what was sent. Both are off
//! unless that test turns them on, and neither is in a build of the app.
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, PoisonError,
};

static READS_ONLY: AtomicBool = AtomicBool::new(false);
static WRITTEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// From now on no visit begins, and every frame written is noted.
pub(super) fn read_only() {
    READS_ONLY.store(true, Ordering::Release);
}

pub(super) fn reads_only() -> bool {
    READS_ONLY.load(Ordering::Acquire)
}

/// One frame is about to be written to a gateway.
pub(super) fn written(frame: &Value) {
    if !reads_only() {
        return;
    }
    // A call by its method; anything else is an answer to a request.
    let what = frame["method"].as_str().unwrap_or("a result frame");
    let mut written = WRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
    written.push(what.to_owned());
}

/// Every frame written so far, by its method.
pub(super) fn all_written() -> Vec<String> {
    WRITTEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
