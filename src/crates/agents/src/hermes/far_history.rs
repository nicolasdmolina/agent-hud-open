//! One read of a far machine's Hermes history: what is asked of the reader
//! program (`far_history/reader.py`), and what its answer must look like
//! to be used (`reply.rs`). Nothing here runs a process or opens a socket:
//! the platform's Hermes channel sends the program over its SSH link.
//!
//! The program runs on the far machine, against that machine's own
//! `state.db`, and never writes there. It is sent on standard input and
//! is not a file on the far disk. Its request is its argument, kept short.
//!
//! Three kinds of read, and a probe that reads nothing:
//!
//! - fingerprints: a page of the `sessions` table by rowid, or of it the
//!   sessions active since a time. A fingerprint is the session's two
//!   activity stamps and the counts this crate's own reader goes by
//!   (`session_counts`), so a session whose fingerprint is unchanged has
//!   nothing new to fetch;
//! - sessions: for a list of ids, each session's row and every one of its
//!   usage rows, with the columns this crate's reader selects. No value
//!   longer than 64 KiB is loaded on the far side: a session that holds one
//!   is too large, and a fingerprint made of one reads `oversized`;
//! - rowids: the rowids alone of a page, which load nothing of a row. A
//!   row whose page cannot be read is named by it and stepped over;
//! - held: `runtime/active_sessions.json`, bounded, as untrusted text.
use super::gateway::MachineKey;
use super::session_counts;
use crate::transcript::{sql_column, sql_columns};
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};

mod reply;
mod rows;

pub use reply::{first_line, parse, First};
pub use rows::{Batch, Cell, Far, No, Print, Prints, Rejected, Reply, Session};

/// The oldest SQLite a far Python may have: the first with `octet_length`,
/// which is how the reader measures a value without loading it. A reader
/// that finds an older one refuses, and reads nothing.
pub const OLDEST_SQLITE: &str = "3.43";

/// The reader, as it is sent.
pub const PROGRAM: &str = include_str!("far_history/reader.py");
/// What the far shell runs. The program arrives on standard input, and
/// `-I` keeps Python from importing anything from the folder it is in or
/// from the user's own packages.
pub const SCRIPT: &str = "exec python3 -I - \"$@\"";
/// The exit status of a shell that found no `python3`.
pub const NO_PYTHON: i32 = 127;
/// The exit status of a reader that ended itself because what it read
/// made its memory grow too far: a value SQLite had to load to measure.
pub const TOO_MUCH_MEMORY: i32 = 5;
/// The start of the argument every reader carries, by which one reader
/// finds another in the far process list.
pub const MARK: &str = "agent-hud-far-history:";
/// The usage rows of one session this crate's reader accepts
/// (`transcript::read`). A session with more is skipped whole.
pub const MOST_USAGE_ROWS: usize = 500;
/// The most a reader returns unless told otherwise.
pub const MOST_OUTPUT: usize = 1 << 20;
/// The most rows one fingerprint page looks at.
pub const MOST_PAGE: u32 = 500;
/// The longest session id asked for. Hermes writes ids of about 22 bytes.
pub const LONGEST_ID: usize = 128;

/// A session row's columns after its rowid and id, as the reader returns
/// them and as `transcript::read` selects them.
pub const SESSION_COLUMNS: [&str; 15] = [
    "started_at",
    "title",
    "cwd",
    "source",
    "model",
    "billing_provider",
    "billing_base_url",
    "billing_mode",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "ended_at",
    "last_activity_at",
    "profile_name",
];
/// A usage row's columns after its session id: its whole identity, its
/// counts and both stamps.
pub const USAGE_COLUMNS: [&str; 11] = [
    "model",
    "billing_provider",
    "billing_base_url",
    "billing_mode",
    "task",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "first_seen",
    "last_seen",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Opens nothing: says whether a reader can start, which it can only
    /// when no other one is alive.
    Probe,
    /// The fingerprints of the sessions among the `limit` rows after
    /// rowid `after`; with `since` (seconds), only of those active from
    /// then on.
    Prints {
        after: i64,
        limit: u32,
        since: Option<f64>,
    },
    /// The rowids alone of the `limit` rows after rowid `after`. Nothing
    /// of a row is loaded for it, so it names a row that cannot be read.
    Rowids {
        after: i64,
        limit: u32,
    },
    Sessions {
        ids: Vec<String>,
    },
    Held,
}

impl Request {
    /// A word for the log.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Prints { since: None, .. } => "prints",
            Self::Prints { .. } => "prints-since",
            Self::Rowids { .. } => "rowids",
            Self::Sessions { .. } => "sessions",
            Self::Held => "held",
        }
    }

    /// Whether the request can be asked at all: a page starts after a
    /// rowid that is not negative. The far program refuses any other too.
    pub fn askable(&self) -> bool {
        !matches!(self, Self::Prints { after, .. } | Self::Rowids { after, .. } if *after < 0)
    }

    /// The request as the program reads it, with the most output it may
    /// return.
    fn argument(&self, most: usize) -> String {
        match self {
            Self::Probe => json!({"k": "probe"}),
            Self::Prints {
                after,
                limit,
                since,
            } => match since {
                Some(since) => {
                    json!({"k": "prints", "after": after, "n": limit, "since": since, "most": most})
                }
                None => json!({"k": "prints", "after": after, "n": limit, "most": most}),
            },
            Self::Rowids { after, limit } => json!({"k": "rowids", "after": after, "n": limit}),
            Self::Sessions { ids } => json!({"k": "sessions", "ids": ids, "most": most}),
            Self::Held => json!({"k": "held"}),
        }
        .to_string()
    }
}

/// Whether `id` can be asked for: some text of a sane length with no
/// control character. Anything else is not an id Hermes writes.
pub fn usable_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= LONGEST_ID && !id.chars().any(char::is_control)
}

/// The argument that marks a reader: the machine's key, then what names
/// `home`, which must be the far home's canonical path (the program checks
/// that it is). A reader that finds another one of the same machine alive
/// opens nothing, whichever of the machine's homes either reads, so the
/// machine's one slot is held on the far side too. So does one that finds
/// another reader of the same home.
pub fn mark(machine: &MachineKey, home: &str) -> String {
    let digest = Sha256::digest(home.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{MARK}{}:{}", machine.as_str(), &hex[..16])
}

/// The program's arguments, for [`SCRIPT`]: its mark, its deadline in
/// seconds, the home, and the request.
pub fn arguments(
    machine: &MachineKey,
    home: &str,
    deadline: Duration,
    most: usize,
    request: &Request,
) -> [String; 4] {
    [
        mark(machine, home),
        format!("{:.3}", deadline.as_secs_f64()),
        home.to_owned(),
        request.argument(most),
    ]
}

/// The SQL for a session's fingerprint as one text, from the columns
/// `sessions` (`s`) and `session_model_usage` (`u`) have: its two activity
/// stamps, then the counts the reader of this crate goes by.
pub fn fingerprint_sql(s: &HashSet<String>, u: &HashSet<String>) -> String {
    format!(
        "IFNULL({},'')||'/'||IFNULL({},'')||'/'||{}",
        sql_column(s, "s", "ended_at", "NULL"),
        sql_column(s, "s", "last_activity_at", "NULL"),
        session_counts::sql(s, u),
    )
}

/// The same fingerprints from a database on this machine: of the `limit`
/// rows after rowid `after`. A mirror's rows give what the far reader gave
/// for them.
pub fn local_prints(db: &Connection, after: i64, limit: u32) -> rusqlite::Result<Vec<Print>> {
    let s = sql_columns(db, "sessions")?;
    let u = sql_columns(db, "session_model_usage")?;
    let mut statement = db.prepare(&format!(
        "SELECT s.rowid,s.id,{} FROM sessions s WHERE s.rowid>?1 ORDER BY s.rowid LIMIT ?2",
        fingerprint_sql(&s, &u)
    ))?;
    let rows = statement.query_map((after, limit), |row| {
        Ok(Print {
            rowid: row.get(0)?,
            id: row.get(1)?,
            print: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests;
