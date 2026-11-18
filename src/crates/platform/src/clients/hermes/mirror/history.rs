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
//! left the list since it last asked.
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
    /// Not written, and nothing of it: the route changed (`route`), or
    /// the machine is retired (`over`).
    Dropped(&'static str),
    Failed(String),
}

/// The live sessions of each profile, as the newest poll listed them,
/// and those that left the list since they were last asked for.
#[derive(Default)]
pub(super) struct Live {
    of: HashMap<Profile, Listing>,
}

#[derive(Default)]
struct Listing {
    read_ms: i64,
    now: BTreeSet<String>,
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
        wrote.recv().unwrap_or(Wrote::Dropped("over"))
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

    /// Whether a poll that came back since `fresh_ms` listed a session.
    pub(crate) fn live(&self, fresh_ms: i64) -> bool {
        let state: std::sync::MutexGuard<'_, State> = self.state();
        let lists = |listing: &Listing| listing.read_ms >= fresh_ms && !listing.now.is_empty();
        state.live.of.values().any(lists)
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
        let (mut written, mut rows) = (0, false);
        let wrote = files.write(profile, &path, Some(batch.route), &*self.routes, |db| {
            let claimed = mirror::claim(db, &self.key, profile, label)?;
            written = mirror::durable(db, &batch.sessions, batch.read_ms)?;
            let held = (batch.held.as_ref()).map_or(Ok(0), |ids| mirror::held(db, ids))?;
            let noted = (batch.reads.as_ref()).map_or(Ok(0), |reads| mirror::note(db, reads))?;
            // Core reads the rows, and a mirror that is new to it.
            rows = written > 0 || claimed;
            Ok(written + held + noted + usize::from(claimed))
        });
        match wrote {
            Ok(Some(_)) => {
                files.holds(
                    profile,
                    batch.sessions.into_iter().map(|session| session.id),
                );
                if rows {
                    (self.changed)(std::slice::from_ref(&path));
                }
                if written > 0 {
                    let line = format!("mirror history profile={name} sessions={written}");
                    self.log.line(&line);
                }
                Wrote::Written(written)
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
