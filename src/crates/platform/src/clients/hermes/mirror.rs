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
//! - **Retired.** Once the machine's last owner is gone (`stop`), no
//!   write begins, the one under way is waited out, and the files are
//!   closed, all before `stop` returns. The mirrors stay where they are,
//!   so the machine's sessions stay listed.
use super::log::Log;
use crate::hooks::Changed;
use agent_hud_agents::hermes::{
    gateway::{Listed, Listing, MachineKey, Profile},
    mirror,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
};

mod files;
#[cfg(test)]
mod tests;
mod work;

/// The most listings that wait. A disk that stalls drops the oldest:
/// every poll lists its sessions anew. A new label is told once, so it
/// is never the one dropped.
const MOST: usize = 64;

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
        let state = State {
            queue: VecDeque::from([Write::Label]),
            label: label.to_owned(),
            over: false,
            busy: false,
            closed: false,
        };
        let writer = Arc::new(Self {
            root: data.join(mirror::ROOT),
            key: key.clone(),
            routes,
            changed,
            log,
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
            let listing = |write: &Write| matches!(write, Write::Listed { .. });
            if state.queue.iter().filter(|write| listing(write)).count() >= MOST {
                let oldest = state.queue.iter().position(listing);
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
