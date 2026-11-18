//! What the tracker decided, kept in the app's data folder across a
//! relaunch. The tracker starts empty and takes only what waits at its
//! first look for old. A request whose card comes back later than that
//! look, from a client the app reaches only after it has started, would
//! be new to it and sound a second time. So every decided identity is
//! saved with when it was last among the inputs, and loaded at start,
//! less the ones older than their one-day lifetime.
//!
//! The file names sessions and requests and holds no word anyone wrote.
//! It is written when something is decided or forgotten, and otherwise
//! seldom: the times in it move at every look.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
};

/// How far the times in the file may fall behind the ones in memory.
const REFRESH_MS: i64 = 600_000;
/// How long before a write that failed is tried again.
const RETRY_MS: i64 = 60_000;
/// The largest file that is read.
const LIMIT: u64 = 4 * 1024 * 1024;

pub(super) struct Kept {
    path: PathBuf,
    /// When the file was last written, or tried.
    tried: i64,
    /// Something was decided or forgotten that the file does not say.
    owed: bool,
    failed: bool,
    /// What the file holds, as it was last written.
    written: Vec<u8>,
}

impl Kept {
    /// The identities the file holds that are younger than `lifetime` at
    /// `now`, and the keeper of the file. A file that is missing, or
    /// cannot be used, holds none.
    pub(super) fn load(path: PathBuf, now: i64, lifetime: i64) -> (Self, HashMap<String, i64>) {
        let small = fs::metadata(&path).is_ok_and(|file| file.len() <= LIMIT);
        let stored: Value = small
            .then(|| fs::read(&path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let entries = stored["decided"]
            .as_object()
            .filter(|_| stored["version"] == 1);
        let decided = entries
            .into_iter()
            .flatten()
            // A time from a clock that ran ahead is not kept past its day.
            .filter_map(|(id, at)| Some((id.clone(), at.as_i64()?.min(now))))
            .filter(|(_, at)| now - *at < lifetime)
            .collect();
        let kept = Self {
            path,
            tried: now,
            owed: false,
            failed: false,
            written: Vec::new(),
        };
        (kept, decided)
    }

    /// Writes `decided` where the file is owed it: at once when the
    /// identities `changed`, and now and then for their times.
    pub(super) fn keep(&mut self, decided: &HashMap<String, i64>, changed: bool, now: i64) {
        self.owed |= changed;
        let wait = match (self.owed, self.failed) {
            (true, false) => 0,
            (true, true) => RETRY_MS,
            (false, _) => REFRESH_MS,
        };
        if now.saturating_sub(self.tried) < wait {
            return;
        }
        self.tried = now;
        let sorted: BTreeMap<&String, &i64> = decided.iter().collect();
        let Ok(bytes) = serde_json::to_vec(&json!({"version":1,"decided":sorted})) else {
            return;
        };
        if !self.owed && bytes == self.written {
            return;
        }
        let temporary = self.path.with_extension("json.tmp");
        let saved = fs::write(&temporary, &bytes).and_then(|_| fs::rename(temporary, &self.path));
        self.failed = saved.is_err();
        if saved.is_ok() {
            self.owed = false;
            self.written = bytes;
        }
    }
}
