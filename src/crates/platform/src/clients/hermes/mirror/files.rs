//! The mirror files a writer has open, and one transaction on one of
//! them. Only the writer's own thread is here.
//!
//! A mirror is in write-ahead mode: core reads it while it is written,
//! and neither waits for the other. Its folders and the file are made
//! for this user alone.
//!
//! A mirror that is not there yet is made under another name, with its
//! first transaction in it, and only then given its own (`make`). Core
//! reads a file by that name alone, so it never finds one without its
//! tables: not in the moment before the first commit, and not after a
//! first write that was given up, which leaves no file at all. A file
//! core could not read it leaves be for 30 s and longer.
use super::Routes;
use agent_hud_agents::hermes::gateway::{Listed, Profile};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

/// How long a write waits for the file: core's reads do not hold it, so
/// this is for a stray process.
const BUSY: Duration = Duration::from_millis(250);
/// What a mirror is called while it is made, in its own folder. Core
/// reads no file of this name.
const MAKING: &str = "state.db.new";

#[derive(Default)]
pub(super) struct Files {
    open: HashMap<Profile, (Connection, PathBuf)>,
    /// When the newest live list taken for each profile was read.
    newest: HashMap<Profile, i64>,
    /// The sessions each mirror is known to hold a row for, so a poll
    /// that lists nothing new opens nothing.
    held: HashMap<Profile, HashSet<String>>,
    /// Why the last write of each mirror failed, where it did, so that
    /// the same failure at every poll is logged once.
    failed: HashMap<Profile, String>,
    /// What its sessions have used so far: the samples that wait to be
    /// written, and what each mirror is known to say (`counts.rs`).
    pub(super) counts: super::counts::Counts,
}

impl Files {
    /// Whether a list of `profile` read at `read_ms` is newer than every
    /// one taken before; it is the newest from now on.
    pub(super) fn newer(&mut self, profile: &Profile, read_ms: i64) -> bool {
        let newest = self.newest.get(profile);
        if newest.is_some_and(|newest| read_ms <= *newest) {
            return false;
        }
        self.newest.insert(profile.clone(), read_ms);
        true
    }

    /// The `sessions` the mirror of `profile` at `path` is not known to
    /// hold. A mirror that is gone from the disk holds none.
    pub(super) fn lacking(
        &mut self,
        profile: &Profile,
        path: &Path,
        sessions: Vec<(String, Listed)>,
    ) -> Vec<(String, Listed)> {
        if !path.is_file() {
            self.forget(profile);
        }
        let held = self.held.get(profile);
        let lacks = |(id, _): &(String, Listed)| !held.is_some_and(|held| held.contains(id));
        sessions.into_iter().filter(lacks).collect()
    }

    /// The mirror of `profile` holds a row for each of `sessions` now:
    /// its write went well.
    pub(super) fn hold(&mut self, profile: &Profile, sessions: Vec<(String, Listed)>) {
        self.failed.remove(profile);
        let held = self.held.entry(profile.clone()).or_default();
        held.extend(sessions.into_iter().map(|(id, _)| id));
    }

    /// The mirror of `profile` holds a row for each of `ids` now.
    pub(super) fn holds(&mut self, profile: &Profile, ids: impl Iterator<Item = String>) {
        self.failed.remove(profile);
        self.held.entry(profile.clone()).or_default().extend(ids);
    }

    /// A write of the mirror of `profile` failed for the reason `why`:
    /// whether that is news, which it is not while every write since the
    /// last good one failed the same way.
    pub(super) fn fails(&mut self, profile: &Profile, why: &str) -> bool {
        let before = self.failed.insert(profile.clone(), why.to_owned());
        before.as_deref() != Some(why)
    }

    /// Nothing is known of the mirror of `profile`: it is opened anew.
    pub(super) fn forget(&mut self, profile: &Profile) {
        self.open.remove(profile);
        self.held.remove(profile);
        self.counts.forget(profile);
    }

    /// One transaction on the mirror of `profile` at `path`, which is
    /// opened where it is not open (`transaction`), and made where it is
    /// not there (`make`).
    pub(super) fn write(
        &mut self,
        profile: &Profile,
        path: &Path,
        route: Option<u64>,
        routes: &dyn Routes,
        body: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
    ) -> Result<Option<usize>, String> {
        if !self.open.contains_key(profile) {
            if !path.is_file() {
                let made = make(path, route, routes, body)?;
                // Open from here on, like one that was there already.
                if path.is_file() {
                    let opened = (open(path)?, path.to_owned());
                    self.open.insert(profile.clone(), opened);
                }
                return Ok(made);
            }
            let opened = (open(path)?, path.to_owned());
            self.open.insert(profile.clone(), opened);
        }
        let (db, _) = self.open.get_mut(profile).expect("just opened");
        transaction(db, route, routes, body)
    }

    /// Closes every mirror that is open; which those were.
    pub(super) fn close(&mut self) -> Vec<PathBuf> {
        self.held.clear();
        self.counts = Default::default();
        let open = self.open.drain();
        open.map(|(_, (_, path))| path).collect()
    }
}

/// One transaction on the mirror at `path`, on a connection of its own.
pub(super) fn transact(
    path: &Path,
    route: Option<u64>,
    routes: &dyn Routes,
    body: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
) -> Result<Option<usize>, String> {
    transaction(&mut open(path)?, route, routes, body)
}

/// Makes the mirror at `path` with its first transaction in it, which
/// `body` writes: under another name, closed so that the file is whole
/// by itself, and then given its own. Where that transaction is not
/// committed, for any reason, no file is left under either name.
fn make(
    path: &Path,
    route: Option<u64>,
    routes: &dyn Routes,
    body: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
) -> Result<Option<usize>, String> {
    let making = path.with_file_name(MAKING);
    let beside = |end: &str| PathBuf::from(format!("{}{end}", making.display()));
    // One a writer that was cut off left, and what SQLite keeps beside.
    let discard = || {
        for end in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(beside(end));
        }
    };
    discard();
    let mut db = open(&making)?;
    let wrote = transaction(&mut db, route, routes, body);
    // Closing folds the write-ahead log into the file.
    let whole = db.close().is_ok() && !beside("-wal").exists();
    if !matches!(wrote, Ok(Some(made)) if made > 0) {
        discard();
        return wrote;
    }
    if !whole || std::fs::rename(&making, path).is_err() {
        discard();
        return Err("file".to_owned());
    }
    wrote
}

/// Opens the mirror at `path`, making its folders and the file where
/// they are not there.
fn open(path: &Path) -> Result<Connection, String> {
    // The mirrors' folder, its `profiles`, and the mirror's own.
    let mut folders: Vec<&Path> = path.ancestors().skip(1).take(3).collect();
    folders.reverse();
    for folder in folders {
        crate::os::private_directory(folder)?;
    }
    drop(crate::os::private_file(path, false).map_err(|_| "file".to_owned())?);
    let db = Connection::open(path).map_err(why)?;
    db.busy_timeout(BUSY).map_err(why)?;
    db.pragma_update(None, "journal_mode", "WAL").map_err(why)?;
    db.pragma_update(None, "synchronous", "NORMAL")
        .map_err(why)?;
    Ok(db)
}

/// Runs `body` in one transaction and commits it where it changed
/// anything, which `body` says by a count that is not 0: then the count.
/// With a `route`, the commit is made only while that is still the
/// machine's route number (`Routes::under`); None where it is not, with
/// nothing written. An error leaves nothing written either.
fn transaction(
    db: &mut Connection,
    route: Option<u64>,
    routes: &dyn Routes,
    body: impl FnOnce(&Connection) -> rusqlite::Result<usize>,
) -> Result<Option<usize>, String> {
    let begun = Transaction::new_unchecked(db, TransactionBehavior::Immediate).map_err(why)?;
    let made = body(&begun).map_err(why)?;
    if made == 0 {
        return Ok(Some(0));
    }
    let mut begun = Some(begun);
    let mut ended = None;
    let mut commit = || ended = begun.take().map(Transaction::commit);
    match route {
        Some(number) if !routes.under(number, &mut commit) => return Ok(None),
        Some(_) => {}
        None => commit(),
    }
    ended.expect("committed").map_err(why)?;
    Ok(Some(made))
}

/// A failure as a log may say it: its kind, and no word of any row.
fn why(error: rusqlite::Error) -> String {
    match error.sqlite_error_code() {
        Some(code) => format!("{code:?}"),
        None => "refused".to_owned(),
    }
}
