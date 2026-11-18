//! One home's part of a tick: a bounded number of reads, each given to
//! one of the three kinds of work (`agents::hermes::far_history::refresh`),
//! and then one write of what the tick learned of the mirror's ages.
//!
//! - **The first queue**: one read of which sessions the desktop app
//!   holds, then the sessions that are live, just ended or held, by id.
//!   What a tick could not get to is first in the next.
//! - **The two sweeps**: a page of fingerprints, held against the
//!   fingerprint each session was last written with; then the sessions
//!   that differ, by id, before the next page. Only those are fetched.
//! - **Nothing is taken for done before it is written.** A session leaves
//!   its queue once the writer has committed it, the far side said it
//!   cannot be read, or the mirror would not take it. A read that failed
//!   and a write that was dropped leave it waiting, and the sweep's
//!   place where it was.
//!
//! A session that cannot be read (too large, too many usage rows), and
//! one the mirror refuses (ticket 61), is remembered with the fingerprint
//! it had, and asked for again only when that changes. Neither stops the
//! sweep, the home, or the machine's other homes.
use super::{History, FRESH};
use crate::clients::hermes::{far_history::Failure, mirror::Batch, mirror::Wrote};
use crate::ssh_link::Link;
use agent_hud_agents::hermes::{
    far_history::refresh::{self, Turn},
    gateway::Profile,
    mirror::{self, Reads},
};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

mod sweep;

/// Why a home's reads stopped before the tick's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stop {
    Read(Failure),
    /// The writer did not take a batch: the route changed, the machine
    /// is retired, or the mirror could not be written.
    Write,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(failure) => write!(f, "{failure}"),
            Self::Write => f.write_str("the mirror did not take the batch"),
        }
    }
}

/// What one home's reads came to.
pub(super) struct Ran {
    pub reads: u32,
    pub overdue: bool,
    pub stopped: Option<Stop>,
}

/// One sweep: where its next page starts, and the sessions its last page
/// found changed, each with the fingerprint it had there.
#[derive(Default)]
struct Pages {
    /// Where the page that found `pending` began.
    from: i64,
    after: i64,
    pending: VecDeque<(String, String)>,
    /// The last page read reached the table's end.
    ended: bool,
    /// The rows a page looks at, where a page of the plan's size took
    /// the far side too long; 0 for the plan's.
    size: u32,
}

/// What a profile's refresh has got to, from tick to tick.
#[derive(Default)]
pub(super) struct Progress {
    /// The mirror's own word of its sweep was read.
    loaded: bool,
    reads: Reads,
    whole: Pages,
    recent: Pages,
    /// The first queue's sessions the last tick did not get to.
    carried: Vec<String>,
    /// Sessions that cannot be read, each with the fingerprint it had
    /// when that was found; empty where that is not known yet.
    skipped: HashMap<String, String>,
    /// Those of them the mirror refused (ticket 63).
    refused: HashSet<String>,
}

impl Progress {
    /// How many sessions the mirror refused and are not counted.
    pub(super) fn refused(&self) -> usize {
        self.refused.len()
    }
}

#[cfg(test)]
impl Progress {
    pub(super) fn sweep(&self) -> (i64, usize) {
        (self.whole.after, self.whole.pending.len())
    }

    pub(super) fn backdate(&mut self, by_ms: i64) {
        self.reads.sweep_done_ms = self.reads.sweep_done_ms.map(|done| done - by_ms);
    }
}

/// One home's reads of one tick.
struct Run<'a> {
    history: &'a History,
    link: &'a Link,
    home: &'a str,
    profile: &'a Profile,
    route: u64,
    /// The fingerprint each durable session of the mirror has.
    stored: HashMap<String, String>,
    at: &'a mut Progress,
    /// When a read of this tick last came back with an answer.
    contact: Option<i64>,
    held: Option<Vec<String>>,
    /// The first queue, once the held sessions were read.
    first: Option<VecDeque<String>>,
    recent_done: bool,
    swept: bool,
}

/// What the mirror at `path` says of itself, read without writing:
/// nothing where it is not there yet.
fn mirrored(path: &Path) -> (HashMap<String, String>, Reads) {
    let read = || -> rusqlite::Result<_> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok((mirror::stored(&db)?, mirror::reads(&db)?))
    };
    match path.is_file() {
        true => read().unwrap_or_default(),
        false => Default::default(),
    }
}

impl History {
    pub(super) fn profile(
        &self,
        link: &Link,
        home: &str,
        profile: &Profile,
        at: &mut Progress,
    ) -> Ran {
        // Before any read: what comes back is written under this number.
        let route = self.route.number();
        let path = mirror::file(&self.root, &self.key, profile);
        let (stored, reads) = mirrored(&path);
        if !std::mem::replace(&mut at.loaded, true) {
            at.reads = reads;
            at.whole.after = reads.sweep_cursor.unwrap_or(0);
            self.state().ages.insert(profile.clone(), reads);
        }
        let mut run = Run {
            history: self,
            link,
            home,
            profile,
            route,
            stored,
            at,
            contact: None,
            held: None,
            first: None,
            recent_done: false,
            swept: false,
        };
        let (mut made, mut stopped) = (0, None);
        for read in 1..=self.plan.reads.max(refresh::SHARE) {
            if self.over() {
                break;
            }
            let overdue = self.overdue(&run.at.reads);
            let first = run.first.as_ref().is_none_or(|queue| !queue.is_empty());
            let did = match refresh::turn(read, overdue, first, !run.recent_done, run.swept) {
                Turn::First => run.first(),
                Turn::Recent => run.page(false),
                Turn::Sweep => run.page(true),
                Turn::Rest => break,
            };
            made += 1;
            if let Err(stop) = did {
                stopped = Some(stop);
                break;
            }
        }
        let stopped = stopped.or(run.finish().err());
        Ran {
            reads: made,
            overdue: self.overdue(&at.reads),
            stopped,
        }
    }
}

impl Run<'_> {
    fn contacted(&mut self) {
        self.contact = Some(crate::hooks::now_ms());
    }

    /// One read of the first queue: the held sessions first, which also
    /// says what the queue is; then a batch of it.
    fn first(&mut self) -> Result<(), Stop> {
        let (history, batch) = (self.history, self.history.plan.batch.max(1));
        let Some(queue) = &mut self.first else {
            let text = (history.reader)
                .held(self.link, &history.slot, self.home)
                .map_err(Stop::Read)?;
            self.contacted();
            let held = text.as_deref().map(refresh::desktop_held);
            let fresh = crate::hooks::now_ms() - FRESH.as_millis() as i64;
            let mut seen = HashSet::new();
            // What the last tick left comes first, so a queue that is
            // never empty still serves every session in turn.
            let queue = (std::mem::take(&mut self.at.carried).into_iter())
                .chain(history.writer.first(self.profile, fresh))
                .chain(held.iter().flatten().cloned())
                .filter(|id| !self.at.skipped.contains_key(id) && seen.insert(id.clone()))
                .collect();
            self.held = Some(held.unwrap_or_default());
            self.first = Some(queue);
            return Ok(());
        };
        let ids: Vec<(String, String)> = (queue.drain(..batch.min(queue.len())))
            .map(|id| (id, String::new()))
            .collect();
        let (unread, did) = self.fetch(&ids);
        let queue = self.first.as_mut().expect("the first queue");
        for (id, _) in unread.into_iter().rev() {
            queue.push_front(id);
        }
        did
    }

    /// The tick's one write of the mirror's own word: its ages, how far
    /// its whole sweep has got, and which sessions are held. Core reads
    /// none of it, and is not told.
    fn finish(mut self) -> Result<(), Stop> {
        let (history, at) = (self.history, &mut *self.at);
        at.carried = self.first.take().map(Vec::from).unwrap_or_default();
        // A page whose sessions are not all written is read again after
        // a relaunch: the sweep's place is where that page began.
        let whole = &at.whole;
        let cursor = match whole.pending.is_empty() {
            true => whole.after,
            false => whole.from,
        };
        at.reads.sweep_cursor = Some(cursor);
        let Some(contact) = self.contact else {
            return Ok(());
        };
        at.reads.contact_ms = Some(contact);
        let reads = at.reads;
        history.state().ages.insert(self.profile.clone(), reads);
        let batch = Batch {
            profile: self.profile.clone(),
            sessions: Vec::new(),
            held: self.held.take(),
            reads: Some(reads),
            read_ms: contact,
            route: self.route,
        };
        match history.writer.history(batch) {
            Wrote::Written(_) | Wrote::Refused(..) => Ok(()),
            Wrote::Dropped(_) | Wrote::Failed(_) => Err(Stop::Write),
        }
    }
}
