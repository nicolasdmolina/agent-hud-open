//! For the test that reads a real far machine and must change nothing
//! there (`tests/far_mini.rs`): a switch that keeps every visit from
//! beginning, since a visit subscribes to a session, for a snapshot as
//! for an answer; and a list of every call written to a gateway, by its
//! method alone, so the test can say exactly what was sent. Both are off
//! unless that test turns them on, and neither is in a build of the app.
//!
//! And for the tests of a poll that comes back late (`tests/far/late.rs`,
//! `tests/link_epoch.rs`): a hold on one poll of one follower, or on one
//! reading of a session apart, between its last reply and the step that
//! hands it to the model (`Late`).
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Mutex, PoisonError,
};

/// Holds the next poll of a follower once it has read everything, until
/// it is let go: a test puts what it likes in between.
#[derive(Default)]
pub(super) struct Late {
    /// What is held next: nothing (0), a poll, or a reading apart.
    armed: AtomicU8,
    held: AtomicBool,
    freed: AtomicBool,
}

const POLL: u8 = 1;
const APART: u8 = 2;

impl Late {
    /// The next poll to come back is held.
    pub(super) fn arm(&self) {
        self.armed.store(POLL, Ordering::Release);
    }

    /// The next reading of a session apart to come back is held.
    pub(super) fn arm_apart(&self) {
        self.armed.store(APART, Ordering::Release);
    }

    /// Whether a poll is held now: it has read all it will.
    pub(super) fn held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    /// Lets the held poll go on to the model.
    pub(super) fn free(&self) {
        self.freed.store(true, Ordering::Release);
    }

    /// A poll has read everything and is about to be handed over.
    pub(super) fn pass(&self) {
        self.wait(POLL);
    }

    /// So has a reading of a session apart.
    pub(super) fn pass_apart(&self) {
        self.wait(APART);
    }

    fn wait(&self, which: u8) {
        let armed = (self.armed).compare_exchange(which, 0, Ordering::AcqRel, Ordering::Acquire);
        if armed.is_err() {
            return;
        }
        self.held.store(true, Ordering::Release);
        while !self.freed.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

static READS_ONLY: AtomicBool = AtomicBool::new(false);
static WRITTEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Every frame written, whole, once a test asks for them (ticket 39's
/// lab with real Hermes, `tests/machines/desktop_real.rs`).
static FRAMES: Mutex<Option<Vec<Value>>> = Mutex::new(None);

/// From now on every frame written to a gateway is kept whole. Visits
/// go on as ever.
pub(super) fn keep_frames() {
    *FRAMES.lock().unwrap_or_else(PoisonError::into_inner) = Some(Vec::new());
}

/// Every frame written since `keep_frames`.
pub(super) fn frames() -> Vec<Value> {
    let kept = FRAMES.lock().unwrap_or_else(PoisonError::into_inner);
    kept.clone().unwrap_or_default()
}

/// From now on no visit begins, and every frame written is noted.
pub(super) fn read_only() {
    READS_ONLY.store(true, Ordering::Release);
}

pub(super) fn reads_only() -> bool {
    READS_ONLY.load(Ordering::Acquire)
}

/// One frame is about to be written to a gateway.
pub(super) fn written(frame: &Value) {
    if let Some(kept) = FRAMES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
    {
        kept.push(frame.clone());
    }
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
