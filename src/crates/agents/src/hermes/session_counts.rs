//! Which Hermes sessions changed since the last full read, by their counts
//! rather than their activity: a call can be written with a stamp earlier
//! than another session's activity a read already went past.
//!
//! Each session's counts are kept on their own: its four token counts, its
//! usage rows' count, each token column's sum over them and their latest
//! `last_seen`. Counts only grow (`transcript::running_totals`), so growth
//! always changes one of them; rows that shrink while others grow by as
//! much are outside that contract. One map of their digests, by rowid,
//! goes from page to page in the cursor and is updated in place.
use crate::transcript::sql_column;
use serde_json::{Map, Value};
use std::collections::HashSet;

/// The SQL for a session's counts as one text, from the columns `sessions`
/// (`s`) and `session_model_usage` (`u`) have.
pub(super) fn sql(s: &HashSet<String>, u: &HashSet<String>) -> String {
    format!(
        "{}||'/'||{}",
        tokens(s, "s", "COALESCE({},0)"),
        if u.contains("session_id") {
            format!(
                "(SELECT COUNT(*)||'/'||{}||'/'||IFNULL(MAX({}),'') FROM session_model_usage u WHERE u.session_id=s.id)",
                tokens(u, "u", "TOTAL({})"),
                sql_column(u, "u", "last_seen", "NULL"),
            )
        } else {
            "''".into()
        }
    )
}

/// The map a pass carries, and the sessions this page read.
pub(super) struct SessionCounts {
    map: Map<String, Value>,
    visited: HashSet<i64>,
}

impl SessionCounts {
    pub(super) fn from_cursor(cursor: &Value) -> Self {
        Self {
            map: cursor["prints"].as_object().cloned().unwrap_or_default(),
            visited: HashSet::new(),
        }
    }

    /// Notes a session's counts; true when they differ from the last read.
    pub(super) fn changed(&mut self, rowid: i64, counts: &str) -> bool {
        self.visited.insert(rowid);
        let digest = Value::from(fnv(counts));
        self.map.insert(rowid.to_string(), digest.clone()) != Some(digest)
    }

    /// The map once this page read the rows after `scan` up to `last`:
    /// sessions gone from those rows, and past `last` once the pass `ended`,
    /// leave it.
    pub(super) fn finish(mut self, scan: i64, last: i64, ended: bool) -> Value {
        let visited = self.visited;
        self.map.retain(|rowid, _| {
            rowid.parse::<i64>().is_ok_and(|rowid| {
                visited.contains(&rowid) || rowid <= scan || (!ended && rowid > last)
            })
        });
        Value::Object(self.map)
    }
}

/// FNV-1a: a stable 64-bit digest of a session's counts, so the map stays
/// small while every count still takes part.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

/// A table's four token counts, each in `wrap` (`{}` is the column) where
/// the table has it, joined into one text.
fn tokens(columns: &HashSet<String>, alias: &str, wrap: &str) -> String {
    [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ]
    .map(|key| wrap.replace("{}", &sql_column(columns, alias, key, "0")))
    .join("||'/'||")
}
