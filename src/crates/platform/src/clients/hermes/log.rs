//! What the connection did, one line each, in `hermes-gateway.log` in the
//! app's data folder: the connection's state, each card as it opens and
//! closes, each answer and what Hermes made of it. A line names a request
//! by its id and never carries a question's words, an answer, a command or
//! a ticket. The followers of a machine's gateways share the file; a line
//! of a gateway other than this machine's own begins with its id.
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, Weak},
};

/// The log starts over past this size, so it never grows without bound.
const LOG_LIMIT: u64 = 256 * 1024;
/// The longest line written; a longer one is cut.
const LINE_LIMIT: usize = 400;

/// One handle on the log. Every follower of a machine has one of the same
/// file, and says who it is at the start of each of its lines.
pub(super) struct Log {
    file: Arc<File>,
    /// What each line of this handle begins with; nothing for the gateway
    /// that is this machine's own.
    who: String,
}

struct File {
    path: PathBuf,
    /// One writer at a time: the size check, the start-over and the write
    /// are one step.
    writer: Mutex<()>,
}

impl Log {
    /// The log in `data`. Every machine that is followed writes the one
    /// file, so each handle on it shares its writer, however it was made.
    pub(super) fn new(data: &Path) -> Self {
        static OPEN: Mutex<Option<HashMap<PathBuf, Weak<File>>>> = Mutex::new(None);
        let path = data.join("hermes-gateway.log");
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        let open = open.get_or_insert_with(HashMap::new);
        open.retain(|_, file| file.strong_count() > 0);
        let file = open.get(&path).and_then(Weak::upgrade).unwrap_or_else(|| {
            let writer = Mutex::new(());
            let file = Arc::new(File {
                path: path.clone(),
                writer,
            });
            open.insert(path, Arc::downgrade(&file));
            file
        });
        let who = String::new();
        Self { file, who }
    }

    /// The same log, each line beginning with `who` where it names one.
    pub(super) fn of(&self, who: Option<&str>) -> Self {
        Self {
            file: self.file.clone(),
            who: who.map(|who| format!("{who} ")).unwrap_or_default(),
        }
    }

    /// Writes `line` as exactly one line, stamped to the millisecond.
    pub(super) fn line(&self, line: &str) {
        let line: String = (self.who.chars())
            .chain(line.chars())
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(LINE_LIMIT)
            .collect();
        let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let line = format!("{stamp} {line}\n");
        let path = &self.file.path;
        let _writer = (self.file.writer.lock()).unwrap_or_else(PoisonError::into_inner);
        let start_over = std::fs::metadata(path).is_ok_and(|file| file.len() > LOG_LIMIT);
        if start_over {
            let _ = std::fs::remove_file(path);
        }
        // Without following a link, and for this user alone.
        if let Ok(mut file) = crate::os::private_append(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}
