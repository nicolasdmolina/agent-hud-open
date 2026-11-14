//! The three reads, each as one call: a page of fingerprints, sessions by
//! id, and the held sessions. A read that does not fit the output bound
//! is asked again in halves. A session is in a batch with all of its usage
//! rows or not at all, so the limit is on sessions and never cuts one.
use super::{Failure, Reader, Slot};
use crate::ssh_link::{quote, Link};
use agent_hud_agents::hermes::far_history::{
    self as far, Prints, Rejected, Reply, Request, Session,
};
use agent_hud_agents::hermes::gateway::MachineKey;
use std::collections::{HashSet, VecDeque};

/// The longest far command line a read is sent as. The link refuses one
/// over 4096 bytes, and a request near 8 KB fails at an `ssh` master.
pub(super) const LONGEST_LINE: usize = 3 * 1024;

/// Why a session was not read. No row of it came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// It has more usage rows than the HUD's reader accepts.
    UsageRows,
    /// It alone does not fit the output bound.
    TooLarge,
    /// Its id is not one Hermes writes, and was not asked for.
    Id,
}

impl Skip {
    pub fn reason(self) -> &'static str {
        match self {
            Self::UsageRows => "more usage rows than the reader accepts",
            Self::TooLarge => "too large for one read",
            Self::Id => "an id that cannot be asked for",
        }
    }
}

/// What reading a list of sessions gave.
#[derive(Debug, Default, PartialEq)]
pub struct Sessions {
    /// Each with every one of its usage rows.
    pub read: Vec<Session>,
    pub skipped: Vec<(String, Skip)>,
    /// Asked for and not in the far database.
    pub gone: Vec<String>,
    /// Why the reads stopped early, and the sessions not asked for yet.
    pub stopped: Option<Failure>,
    pub unread: Vec<String>,
}

/// The counts of an answer, for its log line.
pub(super) fn counts(reply: &Reply) -> String {
    match reply {
        Reply::Prints(page) => format!(" prints={} seen={}", page.prints.len(), page.seen),
        Reply::Rowids(found) => format!(" rowids={}", found.len()),
        Reply::Sessions(batch) => format!(
            " sessions={} usage={} skipped={}",
            batch.sessions.len(),
            batch
                .sessions
                .iter()
                .map(|session| session.usage.len())
                .sum::<usize>(),
            batch.skipped.len()
        ),
        Reply::Held(held) => format!(" held={}", if held.is_some() { "some" } else { "none" }),
        Reply::TooLarge => " too large".into(),
        _ => String::new(),
    }
}

/// The length of the line the link sends for a read's arguments.
pub(super) fn line_length(args: &[String]) -> usize {
    let words: usize = args.iter().map(|arg| 1 + quote(arg).len()).sum();
    "exec sh -c ".len() + quote(far::SCRIPT).len() + " sh".len() + words
}

impl Reader {
    /// The ids in reads of at most `batch` sessions, each short enough to
    /// send. An id that alone is too long still gets a read of its own,
    /// which the link refuses.
    pub(super) fn reads(
        &self,
        machine: &MachineKey,
        home: &str,
        ids: Vec<String>,
    ) -> VecDeque<Vec<String>> {
        let settings = self.settings();
        let fits = |ids: &[String]| {
            let request = Request::Sessions { ids: ids.to_vec() };
            let args = far::arguments(
                machine,
                home,
                settings.deadline,
                settings.most_output,
                &request,
            );
            ids.len() <= settings.batch.max(1) && line_length(&args) <= LONGEST_LINE
        };
        let mut reads = VecDeque::new();
        let mut read: Vec<String> = Vec::new();
        for id in ids {
            read.push(id);
            if read.len() > 1 && !fits(&read) {
                let id = read.pop().unwrap_or_default();
                reads.push_back(std::mem::replace(&mut read, vec![id]));
            }
        }
        if !read.is_empty() {
            reads.push_back(read);
        }
        reads
    }

    /// Each session's row and every one of its usage rows. A read that is
    /// too large is asked again in halves; a session that is too large by
    /// itself, or has too many usage rows, is skipped with why. A read
    /// that fails stops the rest, and what came whole before it is kept.
    pub fn sessions(&self, link: &Link, slot: &Slot, home: &str, ids: &[String]) -> Sessions {
        let mut out = Sessions::default();
        let mut asked = HashSet::new();
        let mut usable = Vec::new();
        for id in ids {
            if !asked.insert(id.as_str()) {
                continue;
            }
            if far::usable_id(id) {
                usable.push(id.clone());
            } else {
                out.skipped.push((id.clone(), Skip::Id));
            }
        }
        let mut reads = self.reads(slot.machine(), home, usable);
        while let Some(ids) = reads.pop_front() {
            let request = Request::Sessions { ids };
            let answer = self.read(link, slot, home, &request);
            let Request::Sessions { ids } = request else {
                continue;
            };
            match answer {
                Ok(Reply::Sessions(batch)) => {
                    let came: HashSet<&str> = batch
                        .sessions
                        .iter()
                        .map(|session| session.id.as_str())
                        .chain(batch.skipped.iter().map(String::as_str))
                        .collect();
                    out.gone
                        .extend(ids.iter().filter(|id| !came.contains(id.as_str())).cloned());
                    for _ in &batch.skipped {
                        let why = Skip::UsageRows.reason();
                        self.say(format!("history sessions skips one session: {why}"));
                    }
                    let skipped = batch.skipped.into_iter().map(|id| (id, Skip::UsageRows));
                    out.skipped.extend(skipped);
                    out.read.extend(batch.sessions);
                }
                Ok(Reply::TooLarge) if ids.len() > 1 => {
                    self.say(format!(
                        "history sessions too large for {} sessions: asks for half",
                        ids.len()
                    ));
                    let (first, second) = ids.split_at(ids.len() / 2);
                    reads.push_front(second.to_vec());
                    reads.push_front(first.to_vec());
                }
                Ok(Reply::TooLarge) => {
                    let why = Skip::TooLarge;
                    self.say(format!(
                        "history sessions skips one session: {}",
                        why.reason()
                    ));
                    out.skipped.extend(ids.into_iter().map(|id| (id, why)));
                }
                other => {
                    out.stopped = Some(other.err().unwrap_or(Failure::Rejected(Rejected::Status)));
                    out.unread = ids.into_iter().chain(reads.drain(..).flatten()).collect();
                }
            }
        }
        out
    }

    /// The fingerprints of the sessions among the `limit` rows after rowid
    /// `after`; with `since` (seconds), only of those active from then on.
    ///
    /// A page that cannot be read is asked again in halves. One row that
    /// cannot be read (the far reader ended itself over a value too long
    /// to hold) is named by a read of rowids alone, which loads nothing of
    /// a row, then read once more by itself, and if it still cannot be
    /// read it comes back as a page that holds no fingerprint and that
    /// rowid as skipped. So the next page starts after it and a sweep goes
    /// on. Nothing of the row is read or logged but its rowid.
    pub fn prints(
        &self,
        link: &Link,
        slot: &Slot,
        home: &str,
        after: i64,
        limit: u32,
        since: Option<f64>,
    ) -> Result<Prints, Failure> {
        let mut limit = limit.clamp(1, far::MOST_PAGE);
        loop {
            let request = Request::Prints {
                after,
                limit,
                since,
            };
            match self.read(link, slot, home, &request)? {
                Reply::Prints(page) => return Ok(page),
                Reply::TooLarge if limit > 1 => limit /= 2,
                Reply::TooLarge => break,
                _ => return Err(Failure::Rejected(Rejected::Status)),
            }
        }
        // Which row that was is asked next, in a read of its own, and
        // a writer may have removed the row in between: the one named
        // now can be the healthy one after it. So the named row is read
        // once more, by itself, and skipped only if it cannot be read
        // either. One row, one more read: nothing here goes round again.
        let next = Request::Rowids { after, limit: 1 };
        let Reply::Rowids(found) = self.read(link, slot, home, &next)? else {
            return Err(Failure::Rejected(Rejected::Status));
        };
        let Some(&rowid) = found.first() else {
            return Ok(Prints {
                last: after,
                ended: true,
                ..Prints::default()
            });
        };
        let alone = Request::Prints {
            after: rowid - 1,
            limit: 1,
            since,
        };
        match self.read(link, slot, home, &alone)? {
            Reply::Prints(page) => Ok(page),
            Reply::TooLarge => {
                self.say(format!(
                    "history prints quarantines rowid {rowid}: too large to read, and stepped over"
                ));
                Ok(Prints {
                    skipped: vec![rowid],
                    seen: 1,
                    last: rowid,
                    ..Prints::default()
                })
            }
            _ => Err(Failure::Rejected(Rejected::Status)),
        }
    }

    /// The far `runtime/active_sessions.json` as it is, to be read as
    /// untrusted text. None where it is missing, too long or not JSON.
    pub fn held(&self, link: &Link, slot: &Slot, home: &str) -> Result<Option<String>, Failure> {
        match self.read(link, slot, home, &Request::Held)? {
            Reply::Held(held) => Ok(held),
            _ => Err(Failure::Rejected(Rejected::Status)),
        }
    }
}
