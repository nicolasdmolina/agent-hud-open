//! What a far machine's own database gave, written into its mirror
//! (ticket 37). Like `write.rs`, nothing here opens a file or begins a
//! transaction: the platform's one mirror writer gives each function the
//! connection it writes on, inside the transaction it commits.
//!
//! - `durable` writes sessions as the far database holds them: each
//!   session's row and every one of its usage rows, with `hud_rows` set
//!   to durable and to the fingerprint the far reader gave. A row the
//!   live list made is replaced in place, by `UPDATE`, so it keeps its
//!   rowid: the reader knows a new session by a rowid above every one it
//!   read, and a row put in again after it was deleted would be taken
//!   for an old one. No `sessions` row is ever deleted.
//! - A session read before what the mirror holds of it is dropped: the
//!   mirror never goes back.
//! - `taken` writes a batch a session at a time. One the mirror will not
//!   take, by a rule of its tables, is left as it was and named, and the
//!   others are written all the same (ticket 61).
//! - `held` notes which sessions the desktop app holds, and `note` the
//!   mirror's two ages and how far its whole sweep has got.
//!
//! The live token counts are another table, written elsewhere, and
//! nothing here reads or writes it.
use crate::hermes::far_history::{Cell, Session, SESSION_COLUMNS, USAGE_COLUMNS};
use rusqlite::{params, params_from_iter, types::Value as Sql, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

/// The usage columns that name a row, which the mirror never holds empty
/// of a value: a far one without is the empty text, as the reader takes
/// it.
const NAMED: usize = 5;

/// One mirror's `hud_reads`: when its far profile was last read at all,
/// where its whole sweep goes on from, and when the sweep under way began
/// and the last one that finished did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub contact_ms: Option<i64>,
    pub sweep_cursor: Option<i64>,
    pub sweep_started_ms: Option<i64>,
    pub sweep_done_ms: Option<i64>,
}

/// Writes `sessions`, read from the far database at `read_ms`, as the
/// mirror's durable rows; how many were written. A session the mirror
/// holds from a read that was no earlier is left as it is. A session
/// whose cells are not in the shape of the column lists is an error, and
/// then nothing of the batch is to be committed.
pub fn durable(db: &Connection, sessions: &[Session], read_ms: i64) -> rusqlite::Result<usize> {
    let set: Vec<String> = (SESSION_COLUMNS.iter().enumerate())
        .map(|(at, column)| format!("{column}=?{}", at + 2))
        .collect();
    let marks = |count: usize| vec!["?"; count].join(",");
    let mut stored = db.prepare("SELECT durable, read_ms FROM hud_rows WHERE session_id=?1")?;
    let mut update = db.prepare(&format!(
        "UPDATE sessions SET {} WHERE id=?1",
        set.join(",")
    ))?;
    let mut insert = db.prepare(&format!(
        "INSERT INTO sessions (id,{}) VALUES ({})",
        SESSION_COLUMNS.join(","),
        marks(SESSION_COLUMNS.len() + 1)
    ))?;
    let mut clear = db.prepare("DELETE FROM session_model_usage WHERE session_id=?1")?;
    let mut usage = db.prepare(&format!(
        "INSERT INTO session_model_usage (session_id,{}) VALUES ({})",
        USAGE_COLUMNS.join(","),
        marks(USAGE_COLUMNS.len() + 1)
    ))?;
    let mut mark = db.prepare(
        "INSERT INTO hud_rows (session_id, durable, read_ms, print) VALUES (?1, 1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET durable=1, read_ms=excluded.read_ms, print=excluded.print",
    )?;
    let mut written = 0;
    for session in sessions {
        let whole = session.cells.len() == SESSION_COLUMNS.len()
            && (session.usage.iter()).all(|row| row.len() == USAGE_COLUMNS.len());
        if !whole {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let held: Option<(i64, i64)> = stored
            .query_row([&session.id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        // What the mirror holds of it was read no earlier: this is old.
        if held.is_some_and(|(durable, at)| durable != 0 && at >= read_ms) {
            continue;
        }
        let id = Sql::Text(session.id.clone());
        let row = || std::iter::once(id.clone()).chain(session.cells.iter().map(Sql::from));
        if update.execute(params_from_iter(row()))? == 0 {
            insert.execute(params_from_iter(row()))?;
        }
        clear.execute([&session.id])?;
        for cells in &session.usage {
            let named = |(at, cell): (usize, &Cell)| match cell {
                Cell::Null if at < NAMED => Sql::Text(String::new()),
                cell => Sql::from(cell),
            };
            let row = std::iter::once(id.clone()).chain(cells.iter().enumerate().map(named));
            usage.execute(params_from_iter(row))?;
        }
        mark.execute(params![session.id, read_ms, session.print])?;
        written += 1;
    }
    Ok(written)
}

/// What became of a batch's sessions in the mirror.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Taken {
    /// How many were written.
    pub written: usize,
    /// The sessions the mirror would not take, each with SQLite's reason.
    pub refused: Vec<(String, String)>,
}

/// Whether `error` is of the session being written and not of the
/// mirror's file: a rule of its tables refused a value, or the session is
/// not in the shape of the column lists. Two far usage rows that come to
/// the same name here are the case known: a name that is a blob there is
/// none here. Anything else (the disk, the file) is the whole batch's.
fn refusal(error: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode::{ConstraintViolation, TooBig, TypeMismatch};
    match error {
        rusqlite::Error::InvalidQuery => true,
        rusqlite::Error::SqliteFailure(failure, _) => {
            matches!(failure.code, ConstraintViolation | TypeMismatch | TooBig)
        }
        _ => false,
    }
}

/// Writes `sessions` as [`durable`] does, each one whole or not at all.
/// A session the mirror refuses is left exactly as the mirror held it and
/// is named in the answer; the rest of the batch is written. An error
/// that is not one session's own is the batch's, and then nothing of it
/// is to be committed.
pub fn taken(db: &Connection, sessions: &[Session], read_ms: i64) -> rusqlite::Result<Taken> {
    let mut taken = Taken::default();
    for session in sessions {
        db.execute_batch("SAVEPOINT hud_session")?;
        match durable(db, std::slice::from_ref(session), read_ms) {
            Ok(written) => taken.written += written,
            Err(error) => {
                db.execute_batch("ROLLBACK TO hud_session; RELEASE hud_session")?;
                if !refusal(&error) {
                    return Err(error);
                }
                taken.refused.push((session.id.clone(), error.to_string()));
                continue;
            }
        }
        db.execute_batch("RELEASE hud_session")?;
    }
    Ok(taken)
}

/// The fingerprint each durable session of the mirror was last written
/// with, by Hermes's id. A session read again with the same one has
/// nothing new.
pub fn stored(db: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut statement = db
        .prepare("SELECT session_id, print FROM hud_rows WHERE durable<>0 AND print IS NOT NULL")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

/// The desktop app holds exactly the sessions `ids`, of those the mirror
/// has a row for; how many rows that changed.
pub fn held(db: &Connection, ids: &[String]) -> rusqlite::Result<usize> {
    let now: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut statement = db.prepare("SELECT session_id FROM hud_rows WHERE held<>0")?;
    let before = statement.query_map([], |row| row.get::<_, String>(0))?;
    let before: HashSet<String> = before.collect::<rusqlite::Result<_>>()?;
    let mut changed = 0;
    for id in before.iter().filter(|id| !now.contains(id.as_str())) {
        changed += db.execute("UPDATE hud_rows SET held=0 WHERE session_id=?1", [id])?;
    }
    for id in now.iter().filter(|id| !before.contains(**id)) {
        changed += db.execute("UPDATE hud_rows SET held=1 WHERE session_id=?1", [id])?;
    }
    Ok(changed)
}

/// The mirror's `hud_reads`; all empty for a mirror that never had one
/// written.
pub fn reads(db: &Connection) -> rusqlite::Result<Reads> {
    let read = db.query_row(
        "SELECT contact_ms, sweep_cursor, sweep_started_ms, sweep_done_ms FROM hud_reads LIMIT 1",
        [],
        |row| {
            Ok(Reads {
                contact_ms: row.get(0)?,
                sweep_cursor: row.get(1)?,
                sweep_started_ms: row.get(2)?,
                sweep_done_ms: row.get(3)?,
            })
        },
    );
    Ok(read.optional()?.unwrap_or_default())
}

/// Sets the mirror's `hud_reads` to `reads`; 1 where that changed it.
pub fn note(db: &Connection, reads: &Reads) -> rusqlite::Result<usize> {
    if self::reads(db)? == *reads {
        return Ok(0);
    }
    let all = params![
        reads.contact_ms,
        reads.sweep_cursor,
        reads.sweep_started_ms,
        reads.sweep_done_ms
    ];
    let set = "contact_ms=?1, sweep_cursor=?2, sweep_started_ms=?3, sweep_done_ms=?4";
    if db.execute(&format!("UPDATE hud_reads SET {set}"), all)? == 0 {
        db.execute("INSERT INTO hud_reads VALUES (?1, ?2, ?3, ?4)", all)?;
    }
    Ok(1)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
