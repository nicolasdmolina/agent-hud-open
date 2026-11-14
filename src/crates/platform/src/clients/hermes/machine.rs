//! One far machine: the route to it, and the followers of its gateways
//! once it is known which machine it is.
//!
//! **Identity before anything is told.** A far machine is named by a key
//! made from what its own Hermes reports and from its root home, so
//! nothing of it can be followed, shown or answered before that is read.
//! So the machine is verified first: its gateways are found (`far.rs`),
//! each is asked for the install's id on a first socket of its own
//! (`route/ssh/first.rs`), and all of them must agree and make a key.
//! Only then are followers made, under that key. A machine whose Hermes
//! reports no usable id, or whose gateways disagree, is not covered and
//! says why; nothing of it reaches core, and it is asked again later.
//!
//! A gateway without the live connection is not followed, and is named
//! in the report. A machine where no gateway has it was never verified,
//! and has nothing to show.
//!
//! Once followed, a machine goes on as this one does: a lost link marks
//! its cards and ends nothing, and the followers find their gateways
//! again when the link is back. It stops being followed only when client
//! hooks turn off, or when the route finds it leads elsewhere: a gateway
//! that reports another install, another root home, or a check on a far
//! home that fails. Its cards are withdrawn then, and it is verified anew.
use super::{
    followers::Followers,
    log::Log,
    route::{ssh::Ssh, Stop},
    Pace,
};
use crate::{
    hooks::Wake,
    ssh_link::{Failure as LinkFailure, Folder, Lost, Settings, Target},
};
use agent_hud_agents::hermes::{gateway::MachineKey, FAR_ROUTE};
use agent_hud_core::requests::Requests;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::Duration,
};

mod follow;
mod verify;

/// The first wait before verifying again, in the waits between two dials
/// of a gateway (`Pace::retry`): half a second. Doubled each time.
const SOON: u32 = 2;
/// The longest wait before verifying again, in the longest waits between
/// two dials (`Pace::retry_limit`): half a minute.
const LATER: u32 = 6;

/// Where a far machine is, and what it is called.
pub struct FarEntry {
    /// The address, as the connection list's reader validated it.
    pub target: Target,
    /// What its sessions and cards say they run on.
    pub label: String,
    /// The one folder that holds the masters of every far machine.
    pub folder: Folder,
    pub settings: Settings,
}

/// How a far machine stands, for whoever lists the machines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// The link is being opened, or the machine is being verified.
    Connecting,
    /// Verified, and its gateways are followed under this key.
    Connected(MachineKey),
    /// The host's key is unknown here, or is not the one that is known.
    NotTrusted,
    /// No link to it just now, and why. It is tried again.
    CannotReach(&'static str),
    /// Reached, and not a machine that can be followed: the reason, as a
    /// word for whoever puts it into the user's language.
    NotCovered(String),
}

pub struct FarObserver {
    inner: Arc<Inner>,
}

pub(super) struct Inner {
    pub(super) route: Arc<Ssh>,
    label: String,
    data: PathBuf,
    requests: Requests,
    wake: Wake,
    pub(super) pace: Pace,
    pub(super) log: Log,
    state: Mutex<State>,
    stopping: Condvar,
}

struct State {
    active: bool,
    standing: Standing,
    followers: Option<Arc<Followers>>,
    /// The gateways found without the live connection: each by its
    /// profile, with Hermes's reason. None of them is followed.
    not_followed: Vec<(String, &'static str)>,
}

impl FarObserver {
    /// Follows the gateways of the far machine at `entry`, logging into
    /// `data`. The link is opened in the background: this returns at once.
    pub fn start(
        entry: FarEntry,
        data: &Path,
        requests: Requests,
        wake: Wake,
    ) -> Result<Self, String> {
        Self::start_at(entry, data, requests, wake, Pace::default())
    }

    pub(crate) fn start_at(
        entry: FarEntry,
        data: &Path,
        requests: Requests,
        wake: Wake,
        pace: Pace,
    ) -> Result<Self, String> {
        if FAR_ROUTE.on(crate::os::OS).value().is_none() {
            return Err("a far Hermes cannot be reached from this system".to_owned());
        }
        let FarEntry {
            target,
            label,
            folder,
            settings,
        } = entry;
        let who = format!("far {}", target.destination());
        let log = Log::new(data).of(Some(&who));
        let said = log.of(Some(&who));
        let says = Arc::new(move |line: &str| said.line(line));
        let route = Arc::new(Ssh::start(folder, target, settings, says)?);
        let state = State {
            active: true,
            standing: Standing::Connecting,
            followers: None,
            not_followed: Vec::new(),
        };
        let inner = Arc::new(Inner {
            route,
            label,
            data: data.to_owned(),
            requests,
            wake,
            pace,
            log,
            state: Mutex::new(state),
            stopping: Condvar::new(),
        });
        let owned = inner.clone();
        let started = thread::Builder::new()
            .name("Hermes far machine".into())
            .spawn(move || owned.work());
        if started.is_err() {
            inner.stop();
            return Err("a far Hermes could not be followed".to_owned());
        }
        inner.log.line("started");
        Ok(Self { inner })
    }

    /// How the machine stands now.
    pub fn standing(&self) -> Standing {
        self.inner.standing()
    }

    /// The gateways found without the live connection, each by its
    /// profile with Hermes's reason. None of them is followed.
    pub fn not_followed(&self) -> Vec<(String, &'static str)> {
        self.inner.state().not_followed.clone()
    }

    #[cfg(test)]
    pub(super) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }
}

impl Drop for FarObserver {
    fn drop(&mut self) {
        self.inner.stop();
    }
}

/// What verifying waits on beside its own work: the observer stopping.
impl Stop for Inner {
    fn wait(&self, time: Duration) -> bool {
        self.pause(time);
        !self.active()
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn active(&self) -> bool {
        self.state().active
    }

    #[cfg(test)]
    pub(super) fn followers(&self) -> Option<Arc<Followers>> {
        self.state().followers.clone()
    }

    pub(super) fn pause(&self, time: Duration) {
        let state = self.state();
        let _ = self
            .stopping
            .wait_timeout_while(state, time, |state| state.active);
    }

    /// A lost link is said first: whatever the machine was, it cannot be
    /// reached just now.
    fn standing(&self) -> Standing {
        match self.route.down() {
            Some(LinkFailure::Lost(Lost::NotTrusted | Lost::KeyChanged)) => Standing::NotTrusted,
            Some(LinkFailure::Lost(lost)) => Standing::CannotReach(lost.reason()),
            Some(LinkFailure::Refused(refused)) => Standing::CannotReach(refused.reason()),
            None => self.state().standing.clone(),
        }
    }

    /// How the machine stands from now on, said once when it changes.
    pub(super) fn stand(&self, standing: Standing) {
        let mut state = self.state();
        if state.standing == standing {
            return;
        }
        state.standing = standing.clone();
        drop(state);
        match standing {
            Standing::NotCovered(why) => self.log.line(&format!("not covered why={why}")),
            Standing::Connected(key) => {
                self.log.line(&format!("followed machine={}", key.as_str()));
            }
            _ => {}
        }
        (self.wake)();
    }

    /// The gateways found without the live connection, said when they
    /// are other ones than before.
    pub(super) fn note_unfollowed(&self, found: Vec<(String, &'static str)>) {
        let mut state = self.state();
        if state.not_followed == found {
            return;
        }
        state.not_followed = found.clone();
        drop(state);
        for (profile, why) in found {
            let line = format!("gateway not followed profile={profile} why={why}");
            self.log.line(&line);
        }
    }

    /// Ends the followers, which withdraw their cards and end their
    /// readers, then the link. No master is left when this returns.
    fn stop(&self) {
        let followers = {
            let mut state = self.state();
            let ran = std::mem::replace(&mut state.active, false);
            if ran {
                self.log.line("stopped");
            }
            state.followers.take()
        };
        self.stopping.notify_all();
        if let Some(followers) = followers {
            followers.stop();
        }
        self.route.stop();
    }

    fn work(self: Arc<Self>) {
        let (soon, later) = (self.pace.retry * SOON, self.pace.retry_limit * LATER);
        let mut wait = soon;
        while self.active() {
            match self.verified() {
                Ok(key) => {
                    wait = soon;
                    self.follow(key);
                }
                Err(standing) => self.stand(standing),
            }
            self.pause(wait);
            wait = (wait * 2).min(later);
        }
    }
}
