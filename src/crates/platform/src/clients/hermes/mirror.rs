//! The one writer of a far machine's mirrors. Core reads sessions from
//! files, one pass at a time under the collection gate, and nothing read
//! over SSH may run under that gate. So what is learned of a far
//! machine's sessions is written to a mirror, a local file in the shape
//! of Hermes's `state.db`, one for each far profile, and core reads the
//! mirror as it reads any Hermes database (`agents::hermes::mirror`).
//!
//! - **One writer.** A machine has one of these, made with it, and it is
//!   the machine's: a route change that leaves the machine an owner
//!   leaves it writing. Every write goes on one queue and one thread
//!   takes them in order. Nothing else writes a mirror, its label
//!   included. Nobody who queues a write waits for it: a follower queues
//!   with its model unlocked, and the thread holds no lock of anyone's
//!   while it writes, but for the route's in the moment it commits.
//! - **What a write carries.** When its data was read, and the route
//!   number it was read under. A write read no later than the one before
//!   it for the same profile is dropped. So is one whose route number is
//!   not the machine's now: that is asked when the write is taken off
//!   the queue and again in the step that commits it (`Routes::under`),
//!   so no write of an old route is committed once the number went up.
//! - **Telling core.** After each committed transaction core is told
//!   which file changed (`hooks::Changed`), which also asks for a pass.
//!   A write that changed nothing commits nothing and tells nothing.
//! - **History.** What the far database itself gave is a write on the
//!   same queue (`history.rs`), which whoever asks waits for: the
//!   refresh reads the next page against what the mirror then holds.
//! - **Counts.** What a round of count calls said the sessions have used
//!   so far (`counting.rs`) goes into `hud_live`, for showing alone
//!   (`mirror::live`), as a write on the same queue. A
//!   count moves with every call a session's model answers, and every
//!   commit asks core for a pass, so counts are not written as they
//!   come: a machine's counts are committed at most once in `SPACE`,
//!   the newest of each session, every profile's at one time and with
//!   one word to core. A count that waits is written when its time
//!   comes whether or not another poll follows. One that says no more
//!   than its mirror holds is not written at all.
//! - **Retired.** Once the machine's last owner is gone (`stop`), no
//!   write begins, the one under way is waited out, and the files are
//!   closed, all before `stop` returns. The mirrors stay where they are,
//!   so the machine's sessions stay listed.
use super::log::Log;
use crate::hooks::Changed;
use agent_hud_agents::hermes::{
    gateway::{Listed, Listing, MachineKey, Profile, Sample},
    mirror,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::Duration,
};

mod counts;
mod files;
mod history;
#[cfg(test)]
mod tests;
mod work;

pub(crate) use history::{Batch, Wrote};

/// The most listings that wait. A disk that stalls drops the oldest:
/// every poll lists its sessions anew. A new label is told once, so it
/// is never the one dropped.
const MOST: usize = 64;

/// The least time between two commits of a machine's counts: what the
/// file watcher's events are held to, so a far session that works asks
/// core for a pass no more often than one on this machine does.
pub(crate) const SPACE: Duration = Duration::from_secs(2);

/// Whether a route number is still a machine's own.
pub(crate) trait Routes: Send + Sync {
    /// Runs `commit` where `number` is the machine's route number now,
    /// which cannot change until `commit` has returned. False, with
    /// `commit` not run, where the route has changed.
    fn under(&self, number: u64, commit: &mut dyn FnMut()) -> bool;
}

pub(crate) struct Writer {
    /// The folder the mirrors are in.
    root: PathBuf,
    key: MachineKey,
    routes: Arc<dyn Routes>,
    changed: Changed,
    log: Log,
    /// The least time between two commits of counts (`SPACE`).
    space: Duration,
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    queue: VecDeque<Write>,
    /// What the machine's sessions show: its owner's label, as it is now.
    label: String,
    /// The machine is retired, or client hooks are off: nothing more is
    /// written.
    over: bool,
    /// A write is under way.
    busy: bool,
    /// The thread has ended, its files closed.
    closed: bool,
    /// The sessions the newest poll listed live, profile by profile.
    live: history::Live,
}

enum Write {
    /// Every mirror of the machine says its label as it is now.
    Label,
    /// The live sessions of one profile, as a poll listed them.
    Listed {
        profile: Profile,
        sessions: Vec<(String, Listed)>,
        read_ms: i64,
        route: u64,
    },
    /// What a round of count calls said the sessions of one profile
    /// have used so far.
    Counted {
        profile: Profile,
        samples: Vec<(String, Sample)>,
        read_ms: i64,
        route: u64,
    },
    /// What the far database gave, and where its answer goes.
    History(Batch, std::sync::mpsc::Sender<Wrote>),
}

impl Writer {
    /// The writer of the mirrors of the machine `key`, in the data
    /// folder `data`. Its mirrors say `label` from the start, whatever
    /// they said when the machine was last followed.
    pub(crate) fn start(
        data: &Path,
        (key, label): (&MachineKey, &str),
        routes: Arc<dyn Routes>,
        changed: Changed,
        log: Log,
    ) -> Result<Arc<Self>, String> {
        Self::spaced(data, (key, label), routes, changed, log, SPACE)
    }

    /// The same, committing counts no more often than `space`.
    pub(crate) fn spaced(
        data: &Path,
        (key, label): (&MachineKey, &str),
        routes: Arc<dyn Routes>,
        changed: Changed,
        log: Log,
        space: Duration,
    ) -> Result<Arc<Self>, String> {
        let state = State {
            queue: VecDeque::from([Write::Label]),
            label: label.to_owned(),
            over: false,
            busy: false,
            closed: false,
            live: history::Live::default(),
        };
        let writer = Arc::new(Self {
            root: data.join(mirror::ROOT),
            key: key.clone(),
            routes,
            changed,
            log,
            space,
            state: Mutex::new(state),
            wake: Condvar::new(),
        });
        let owned = writer.clone();
        thread::Builder::new()
            .name("Hermes mirror".into())
            .spawn(move || owned.work())
            .map_err(|_| "the Hermes mirror could not be written".to_owned())?;
        Ok(writer)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A poll that counted listed these live sessions, over a socket
    /// opened under the route number `route`. It returns at once.
    pub(crate) fn listed(&self, listing: Listing, route: u64) {
        let read_ms = listing.read_ms;
        let mut state = self.state();
        if state.over {
            return;
        }
        for (profile, sessions) in listing.profiles {
            state.live.listed(&profile, &sessions, read_ms);
            let listed = |write: &Write| matches!(write, Write::Listed { .. });
            if state.queue.iter().filter(|write| listed(write)).count() >= MOST {
                let oldest = state.queue.iter().position(listed);
                oldest.and_then(|oldest| state.queue.remove(oldest));
            }
            state.queue.push_back(Write::Listed {
                profile,
                sessions,
                read_ms,
                route,
            });
        }
        drop(state);
        self.wake.notify_all();
    }

    /// A round of count calls read at `read_ms`, over a socket opened
    /// under the route number `route`, said these sessions have used
    /// this much so far. It returns at once. At most one such write
    /// waits for a profile: a newer round says what the one before it
    /// said, or more.
    pub(crate) fn counted(
        &self,
        samples: BTreeMap<Profile, Vec<(String, Sample)>>,
        read_ms: i64,
        route: u64,
    ) {
        let mut state = self.state();
        if state.over {
            return;
        }
        for (profile, samples) in samples {
            let same = |write: &Write| matches!(write, Write::Counted { profile: of, route: under, .. } if *of == profile && *under == route);
            let waits = state.queue.iter().position(same);
            let mut samples = samples;
            if let Some(Write::Counted {
                samples: before, ..
            }) = waits.and_then(|at| state.queue.remove(at))
            {
                // What the older round said of a session this one did
                // not ask is kept.
                let asked = |id: &String| samples.iter().any(|(now, _)| now == id);
                let kept: Vec<_> = before.into_iter().filter(|(id, _)| !asked(id)).collect();
                samples.extend(kept);
            }
            state.queue.push_back(Write::Counted {
                profile,
                samples,
                read_ms,
                route,
            });
        }
        drop(state);
        self.wake.notify_all();
    }

    /// The machine goes by `label` from now on.
    pub(crate) fn relabel(&self, label: &str) {
        let mut state = self.state();
        if state.over || state.label == label {
            return;
        }
        state.label = label.to_owned();
        // The write takes the label as it is when its turn comes, so one
        // that waits already serves.
        if !state
            .queue
            .iter()
            .any(|write| matches!(write, Write::Label))
        {
            state.queue.push_back(Write::Label);
        }
        drop(state);
        self.wake.notify_all();
    }

    /// Nothing more is written: what waits is dropped, and when this
    /// returns a write that was under way has ended and every mirror is
    /// closed. It waits on this machine's disk and on nothing else.
    pub(crate) fn stop(&self) {
        let mut state = self.state();
        state.over = true;
        state.queue.clear();
        self.wake.notify_all();
        while !state.closed {
            state = (self.wake.wait(state)).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Waits until everything queued so far is written or dropped, for
    /// the tests that look at a mirror afterwards.
    #[cfg(test)]
    pub(crate) fn settle(&self) {
        let mut state = self.state();
        while state.busy || !(state.queue.is_empty() || state.over) {
            state = (self.wake.wait(state)).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The mirror of `profile`, for the tests that read one.
    #[cfg(test)]
    pub(crate) fn file(&self, profile: &Profile) -> PathBuf {
        mirror::file(&self.root, &self.key, profile)
    }
}
