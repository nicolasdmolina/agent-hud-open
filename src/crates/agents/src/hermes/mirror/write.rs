//! Writing a mirror: where its file is, the tables it has, and the three
//! things that are written into one so far. Nothing here opens a file,
//! begins a transaction or runs a thread: the platform's Hermes channel
//! has the one writer of each machine's mirrors
//! (`platform/src/clients/hermes/mirror.rs`), and gives each function the
//! connection it writes on.
//!
//! - `claim` makes a file a mirror of one far profile, or finds that it
//!   is one already: Hermes's `sessions` and `session_model_usage` with
//!   the columns the reader selects, and the four `hud_` tables.
//! - `relabel` keeps `hud_mirror.label` as the machine is called now.
//! - `list` gives a row to each session a poll listed that the mirror
//!   lacks: what the list said of it, no token count, and `durable` 0 in
//!   `hud_rows`. It never touches a row that is there, so it can neither
//!   overwrite what the far database gave nor change a row it made
//!   before.
use super::identity;
use crate::hermes::far_history::{SESSION_COLUMNS, USAGE_COLUMNS};
use crate::hermes::gateway::{Listed, MachineKey, Profile};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// Where the mirrors are, in Agent HUD's own data folder. The source of
/// the Hermes adapter names the same folder (`hermes/mod.rs`).
pub const ROOT: &str = "clients/hermes-far";
/// The folder of `ROOT` that holds one folder for each mirror. The
/// shared walker lists a Hermes database only in a folder under one of
/// this name (`core/src/log_files/walk.rs`).
const PROFILES: &str = "profiles";
const DATABASE: &str = "state.db";

/// The mirror of `profile` of the machine `key`, under the mirrors'
/// folder `root`. The path says nothing to the reader, which goes by the
/// `hud_mirror` table.
pub fn file(root: &Path, key: &MachineKey, profile: &Profile) -> PathBuf {
    let folder = format!("{}-{}", key.as_str(), profile.name());
    root.join(PROFILES).join(folder).join(DATABASE)
}

/// Every mirror of the machine `key` there is under `root`, whichever
/// profile it is of.
pub fn files(root: &Path, key: &MachineKey) -> Vec<PathBuf> {
    let its = format!("{}-", key.as_str());
    let Ok(folders) = std::fs::read_dir(root.join(PROFILES)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = folders
        .flatten()
        .filter(|folder| folder.file_name().to_string_lossy().starts_with(&its))
        .map(|folder| folder.path().join(DATABASE))
        .filter(|file| file.is_file())
        .collect();
    found.sort();
    found
}

/// The tables of a mirror. Hermes's two with the columns the reader
/// selects, typed as Hermes types them and with nothing required that a
/// live list may not say; then the four of Agent HUD's own.
fn tables() -> String {
    let typed = |column: &&str| {
        let kind = match *column {
            "started_at" | "ended_at" | "last_activity_at" | "first_seen" | "last_seen" => "REAL",
            name if name.ends_with("_tokens") => "INTEGER",
            _ => "TEXT",
        };
        format!("{column} {kind}")
    };
    let session: Vec<String> = SESSION_COLUMNS.iter().map(typed).collect();
    // A usage row is named by all of these, as in Hermes.
    let named = [
        "model",
        "billing_provider",
        "billing_base_url",
        "billing_mode",
        "task",
    ];
    let usage: Vec<String> = (USAGE_COLUMNS.iter())
        .map(|column| match named.contains(column) {
            true => format!("{column} TEXT NOT NULL DEFAULT ''"),
            false => typed(column),
        })
        .collect();
    format!(
        "CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, {});
         CREATE TABLE IF NOT EXISTS session_model_usage (session_id TEXT NOT NULL, {},
            PRIMARY KEY (session_id, {}));
         CREATE TABLE IF NOT EXISTS hud_mirror (schema INTEGER NOT NULL, machine TEXT NOT NULL,
            label TEXT NOT NULL, profile TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS hud_rows (session_id TEXT PRIMARY KEY, durable INTEGER NOT NULL,
            read_ms INTEGER NOT NULL, print TEXT, held INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS hud_live (session_id TEXT PRIMARY KEY,
            input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL,
            read_ms INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS hud_reads (contact_ms INTEGER, sweep_cursor INTEGER,
            sweep_started_ms INTEGER, sweep_done_ms INTEGER);",
        session.join(", "),
        usage.join(", "),
        named.join(", "),
    )
}

/// Makes `db` the mirror of `profile` of the machine `key`, shown as
/// `label`, or finds that it is; true where anything was written. An
/// error, and nothing written, for a database that is another machine's
/// or another profile's mirror, or one the reader would not trust
/// afterwards (`identity`): no label, say.
pub fn claim(
    db: &Connection,
    key: &MachineKey,
    profile: &Profile,
    label: &str,
) -> rusqlite::Result<bool> {
    let refused = || rusqlite::Error::InvalidQuery;
    if label.is_empty() {
        return Err(refused());
    }
    let made = !db
        .prepare("SELECT 1 FROM sqlite_master WHERE name='hud_mirror'")?
        .exists([])?;
    let mut changed = made;
    if made {
        db.execute_batch(&tables())?;
        db.execute(
            "INSERT INTO hud_mirror VALUES (1, ?1, ?2, ?3)",
            params![key.as_str(), label, profile.name()],
        )?;
        db.execute("INSERT INTO hud_reads DEFAULT VALUES", [])?;
    }
    let its = identity(db)?.ok_or_else(refused)?;
    if its.key != key.stored() || its.profile != *profile {
        return Err(refused());
    }
    if its.label != label {
        changed |= db.execute("UPDATE hud_mirror SET label=?1", [label])? > 0;
    }
    Ok(changed)
}

/// The sessions of `db`, a mirror of the machine `key`, show `label`
/// from now on; true where it was another. An error, and nothing
/// written, for anything but a mirror of that machine the reader trusts.
pub fn relabel(db: &Connection, key: &MachineKey, label: &str) -> rusqlite::Result<bool> {
    let refused = || rusqlite::Error::InvalidQuery;
    let its = identity(db)?.ok_or_else(refused)?;
    if its.key != key.stored() || label.is_empty() {
        return Err(refused());
    }
    let changed = db.execute("UPDATE hud_mirror SET label=?1 WHERE label<>?1", [label])?;
    Ok(changed > 0)
}

/// Gives a row to each of `listed`, the sessions a poll read at
/// `read_ms` found live, that the mirror has none for; how many it made.
/// A session whose list gave no start is taken to have started when it
/// was read.
pub fn list(db: &Connection, listed: &[(String, Listed)], read_ms: i64) -> rusqlite::Result<usize> {
    let mut has = db.prepare(
        "SELECT 1 FROM sessions WHERE id=?1 UNION ALL SELECT 1 FROM hud_rows WHERE session_id=?1",
    )?;
    let mut session = db.prepare(
        "INSERT INTO sessions (id, title, model, source, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut row =
        db.prepare("INSERT INTO hud_rows (session_id, durable, read_ms) VALUES (?1, 0, ?2)")?;
    let mut made = 0;
    for (id, said) in listed {
        if has.exists([id])? {
            continue;
        }
        let started = said.started_at.unwrap_or(read_ms as f64 / 1000.0);
        session.execute(params![id, said.title, said.model, said.source, started])?;
        row.execute(params![id, read_ms])?;
        made += 1;
    }
    Ok(made)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
