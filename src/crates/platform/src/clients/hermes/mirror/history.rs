//! The writer's part in a far machine's history (ticket 37): what the
//! far database gave goes into the mirror through the same queue and the
//! same thread as everything else, in one transaction for each batch.
//!
//! Whoever asks waits for the answer (`Writer::history`), since the
//! refresh compares the next fingerprints against what the mirror holds
//! by then. It waits on this machine's disk and on nothing else, and a
//! writer that is stopped answers at once.
//!
//! The writer also knows which sessions are live: every poll that counted
//! lists them, and the list passes through here on its way to the rows
//! (`Writer::listed`). The refresh asks for them, with the ones that
//! left the list since it last asked. And it knows when a poll last
//! found one of them in a turn (`Writer::worked`), which is what the
//! refresh's pace goes by: a session that is only listed, idle, does not
//! hurry it (ticket 61).
use super::{files::Files, State, Write, Writer};
use agent_hud_agents::hermes::{
    far_history::Session,
    gateway::{Listed, Profile},
    mirror::{self, Reads},
};
use std::collections::{BTreeSet, HashMap};

/// One write of what the far database gave of one profile.
pub(crate) struct Batch {
    pub profile: Profile,
    /// Each with every one of its usage rows.
    pub sessions: Vec<Session>,
    /// The sessions the desktop app holds, where that was read.
    pub held: Option<Vec<String>>,
    /// The mirror's ages and its sweep's place, where they changed.
    pub reads: Option<Reads>,
    /// When the read that gave the sessions began.
    pub read_ms: i64,
    /// The route number it was read under.
    pub route: u64,
}

/// What became of a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Wrote {
    /// Committed, or there was nothing to change: how many sessions were
    /// written. A session the mirror held from a later read is not one.
    Written(usize),
    /// Committed as `Written`, but for the sessions named: the mirror
    /// would not take them, and holds of each what it held before.
    Refused(usize, Vec<String>),
    /// Not written, and nothing of it: the route changed (`route`), or
    /// the machine is retired (`over`).
    Dropped(&'static str),
    Failed(String),
}

/// The live sessions of each profile, as the newest poll listed them,
/// and those that left the list since they were last asked for.
#[derive(Default)]
pub(super) struct Live {
    pub(super) of: HashMap<Profile, Listing>,
    /// When a poll last came back with a session in a turn.
    worked_ms: Option<i64>,
    /// The sessions the desktop app holds, as the refresh last read them
    /// (ticket 39, `desktop.rs`).
    pub(super) held: HashMap<Profile, BTreeSet<String>>,
}

#[derive(Default)]
pub(super) struct Listing {
    pub(super) read_ms: i64,
    pub(super) now: BTreeSet<String>,
    ended: BTreeSet<String>,
}

impl Live {
    /// A poll that came back at `read_ms` listed `sessions` in `profile`.
    pub(super) fn listed(
        &mut self,
        profile: &Profile,
        sessions: &[(String, Listed)],
        read_ms: i64,
    ) {
        let listing = self.of.entry(profile.clone()).or_default();
        if read_ms <= listing.read_ms {
            return;
        }
        let now: BTreeSet<String> = sessions.iter().map(|(id, _)| id.clone()).collect();
        let gone = std::mem::replace(&mut listing.now, now);
        let now = &listing.now;
        (listing.ended).extend(gone.into_iter().filter(|id| !now.contains(id)));
        listing.read_ms = read_ms;
    }
}

impl Writer {
    /// Writes `batch` and says what became of it. It waits for its turn
    /// on the queue and for the write, never for the network.
    pub(crate) fn history(&self, batch: Batch) -> Wrote {
        let held = (batch.held.clone()).map(|ids| (batch.profile.clone(), ids));
        let (done, wrote) = std::sync::mpsc::channel();
        {
            let mut state = self.state();
            if state.over {
                return Wrote::Dropped("over");
            }
            state.queue.push_back(Write::History(batch, done));
        }
        self.wake.notify_all();
        // A writer that stops drops what waits, and the sender with it.
        let wrote = wrote.recv().unwrap_or(Wrote::Dropped("over"));
        if let (Wrote::Written(_), Some((profile, ids))) = (&wrote, held) {
            self.holds(profile, ids);
        }
        wrote
    }

    /// The sessions of `profile` the refresh reads first: those the
    /// newest poll listed live, and those that left the list since this
    /// was last asked, which are then forgotten. A list read before
    /// `fresh_ms` says nothing of what is live now: its sessions are
    /// given once more, as ended.
    pub(crate) fn first(&self, profile: &Profile, fresh_ms: i64) -> Vec<String> {
        let mut state = self.state();
        let Some(listing) = state.live.of.get_mut(profile) else {
            return Vec::new();
        };
        let mut ids = std::mem::take(&mut listing.ended);
        match listing.read_ms >= fresh_ms {
            true => ids.extend(listing.now.iter().cloned()),
            false => ids.append(&mut listing.now),
        }
        ids.into_iter().collect()
    }

    /// A poll that came back at `read_ms` found a session of the machine
    /// in a turn.
    pub(crate) fn worked(&self, read_ms: i64) {
        let mut state: std::sync::MutexGuard<'_, State> = self.state();
        state.live.worked_ms = state.live.worked_ms.max(Some(read_ms));
    }

    /// Whether a poll that came back since `fresh_ms` found a session in
    /// a turn. Sessions that are listed and idle do not count, however
    /// many: a gateway may list its old chats for as long as it runs.
    pub(crate) fn working(&self, fresh_ms: i64) -> bool {
        (self.state().live.worked_ms).is_some_and(|worked| worked >= fresh_ms)
    }

    /// One batch, as one transaction on its profile's mirror, which is
    /// made where it is not there. On the writer's own thread.
    pub(super) fn keep(&self, files: &mut Files, batch: Batch, label: &str) -> Wrote {
        let (profile, name) = (&batch.profile, batch.profile.name());
        if !self.routes.under(batch.route, &mut || {}) {
            let line = format!("mirror dropped profile={name} why=route history");
            self.log.line(&line);
            return Wrote::Dropped("route");
        }
        let path = mirror::file(&self.root, &self.key, profile);
        let (mut written, mut rows, mut refused) = (0, false, Vec::new());
        let wrote = files.write(profile, &path, Some(batch.route), &*self.routes, |db| {
            let claimed = mirror::claim(db, &self.key, profile, label)?;
            // A session at a time: one the mirror will not take does not
            // cost the others their write.
            let taken = mirror::taken(db, &batch.sessions, batch.read_ms)?;
            (written, refused) = (taken.written, taken.refused);
            let held = (batch.held.as_ref()).map_or(Ok(0), |ids| mirror::held(db, ids))?;
            let noted = (batch.reads.as_ref()).map_or(Ok(0), |reads| mirror::note(db, reads))?;
            // Core reads the rows, and a mirror that is new to it.
            rows = written > 0 || claimed;
            Ok(written + held + noted + usize::from(claimed))
        });
        match wrote {
            Ok(Some(_)) => {
                let not = |id: &String| refused.iter().all(|(refused, _)| refused != id);
                let ids = batch.sessions.into_iter().map(|session| session.id);
                files.holds(profile, ids.filter(not));
                if rows {
                    (self.changed)(std::slice::from_ref(&path));
                }
                if written > 0 {
                    let line = format!("mirror history profile={name} sessions={written}");
                    self.log.line(&line);
                }
                for (id, why) in &refused {
                    let line = format!("mirror refused profile={name} session={id} why={why}");
                    self.log.line(&line);
                }
                match refused.is_empty() {
                    true => Wrote::Written(written),
                    false => {
                        Wrote::Refused(written, refused.into_iter().map(|(id, _)| id).collect())
                    }
                }
            }
            // The route changed between the look above and the commit.
            Ok(None) => {
                let line = format!("mirror dropped profile={name} why=route history");
                self.log.line(&line);
                Wrote::Dropped("route")
            }
            Err(why) => {
                files.forget(profile);
                if files.fails(profile, &why) {
                    let line = format!("mirror failed profile={name} why={why}");
                    self.log.line(&line);
                }
                Wrote::Failed(why)
            }
        }
    }
}
