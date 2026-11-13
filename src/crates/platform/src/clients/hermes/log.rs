//! What the connection did, one line each, in `hermes-gateway.log` in the
//! app's data folder: the connection's state, each card as it opens and
//! closes, each answer and what Hermes made of it. A line names a request
//! by its id and never carries a question's words, an answer, a command or
//! a ticket.
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

/// The log starts over past this size, so it never grows without bound.
const LOG_LIMIT: u64 = 256 * 1024;
/// The longest line written; a longer one is cut.
const LINE_LIMIT: usize = 400;

pub(super) struct Log {
    path: PathBuf,
    /// One writer at a time: the size check, the start-over and the write
    /// are one step.
    writer: Mutex<()>,
}

impl Log {
    pub(super) fn new(data: &Path) -> Self {
        Self {
            path: data.join("hermes-gateway.log"),
            writer: Mutex::new(()),
        }
    }

    /// Writes `line` as exactly one line, stamped to the millisecond.
    pub(super) fn line(&self, line: &str) {
        let line: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(LINE_LIMIT)
            .collect();
        let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let line = format!("{stamp} {line}\n");
        let _writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let start_over = std::fs::metadata(&self.path).is_ok_and(|file| file.len() > LOG_LIMIT);
        if start_over {
            let _ = std::fs::remove_file(&self.path);
        }
        // Without following a link, and for this user alone.
        if let Ok(mut file) = crate::os::private_append(&self.path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
