//! One entry's route to a far machine: the link to its address, and the
//! thread that finds out which machine that address leads to.
//!
//! **Identity before anything is told.** A far machine is named by a key
//! made from what its own Hermes reports and from its root home, so
//! nothing of it can be followed, shown or answered before that is read.
//! So the route is verified first: its gateways are found (`far.rs`),
//! each is asked for the install's id on a first socket of its own
//! (`route/ssh/first.rs`), and all of them must agree and make a key. A
//! route whose Hermes reports no usable id, or whose gateways disagree,
//! is not covered and says why, and is asked again later. A gateway
//! without the live connection is named in the report; a route where no
//! gateway has it verifies as nothing.
//!
//! **A route is an entry's, a machine is not.** What a route verified as
//! is told to whoever keeps the machines (`Keeper`, `machines/book.rs`),
//! who says what the route is to that machine. Its owner: the machine's
//! followers run over it, and it watches for word that it leads there no
//! more (a gateway that reports another install, another root home, a
//! check on a far home that fails), which it tells. Or another entry's
//! route owns the machine already: then this one is an alias, follows
//! nothing, and waits to be asked to verify again. A lost link ends
//! neither: it is opened again, for as long as it takes.
use super::{log::Log, route::ssh::Ssh, Pace};
use crate::ssh_link::{Failure as LinkFailure, Folder, Lost, Settings, Target};
use agent_hud_agents::hermes::{gateway::MachineKey, FAR_ROUTE};
use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak},
    thread,
    time::Duration,
};

mod alone;
mod follow;
mod verify;

pub use alone::FarObserver;

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

/// How a route stands, for whoever lists the machines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// The link is being opened, or the machine is being verified.
    Connecting,
    /// Verified, and the gateways of this machine are followed over it.
    Connected(MachineKey),
    /// Verified as this machine, which is followed over another entry's
    /// route, or is the machine Agent HUD runs on. Nothing is followed.
    Alias(MachineKey),
    /// The host's key is unknown here, or is not the one that is known.
    NotTrusted,
    /// No link to it just now, and why. It is tried again.
    CannotReach(&'static str),
    /// Reached, and not a machine that can be followed: the reason, as a
    /// word for whoever puts it into the user's language.
    NotCovered(String),
}

/// What a route is to the machine it verified as.
pub(super) enum Role {
    /// The machine's followers run over it from now on.
    Owner,
    /// Another route owns the machine, or it is this one: nothing runs.
    Alias,
    /// The route is its entry's no more. It ends.
    Over,
}

/// Who keeps the machines the routes lead to. Each is told by the
/// route's own thread, with nothing of the route locked.
pub(super) trait Keeper: Send + Sync {
    /// `route` verified, just now, as the machine `key` names.
    fn verified(&self, route: &Inner, key: &MachineKey) -> Role;
    /// `route` was looked at and verified as no machine: how it stands.
    fn unverified(&self, route: &Inner, standing: &Standing);
    /// `route`, which owned the machine `key`, leads to it no more.
    fn left(&self, route: &Inner, key: &MachineKey);
    /// How a route stands has changed.
    fn changed(&self);
    /// The followers of the machine `route` owns.
    #[cfg(test)]
    fn followers(&self, route: &Inner) -> Option<Arc<super::followers::Followers>>;
}

pub(super) struct Inner {
    pub(super) route: Arc<Ssh>,
    keeper: Weak<dyn Keeper>,
    pub(super) pace: Pace,
    pub(super) log: Log,
    state: Mutex<State>,
    stopping: Condvar,
}

struct State {
    active: bool,
    standing: Standing,
    /// Asked to look again at once: an alias to verify, an owner that owns
    /// no more.
    stirred: bool,
    /// The gateways found without the live connection: each by its
    /// profile, with Hermes's reason. None of them is followed.
    not_followed: Vec<(String, &'static str)>,
}

impl Inner {
    /// An entry's route to `target`, logging into `data`.
    /// The link is opened in the background: this returns at once.
    pub(super) fn start(
        target: Target,
        (folder, settings): (Folder, Settings),
        keeper: Weak<dyn Keeper>,
        (data, pace): (&Path, Pace),
    ) -> Result<Arc<Self>, String> {
        if FAR_ROUTE.on(crate::os::OS).value().is_none() {
            return Err("a far Hermes cannot be reached from this system".to_owned());
        }
        let who = format!("far {}", target.destination());
        let log = Log::new(data).of(Some(&who));
        let said = log.of(Some(&who));
        let says = Arc::new(move |line: &str| said.line(line));
        let route = Arc::new(Ssh::start(folder, target, settings, says)?);
        let state = State {
            active: true,
            standing: Standing::Connecting,
            stirred: false,
            not_followed: Vec::new(),
        };
        let inner = Arc::new(Inner {
            route,
            keeper,
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
        Ok(inner)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn active(&self) -> bool {
        self.state().active
    }

    /// Whoever keeps the machines, while there is one.
    pub(super) fn keeper(&self) -> Option<Arc<dyn Keeper>> {
        self.keeper.upgrade()
    }

    #[cfg(test)]
    pub(super) fn followers(&self) -> Option<Arc<super::followers::Followers>> {
        self.keeper()?.followers(self)
    }

    /// The gateways found without the live connection, each by its
    /// profile with Hermes's reason. None of them is followed.
    pub(super) fn not_followed(&self) -> Vec<(String, &'static str)> {
        self.state().not_followed.clone()
    }

    /// Waits `time`, or less where the route stops or is asked to look.
    pub(super) fn pause(&self, time: Duration) {
        let state = self.state();
        let waits = |state: &mut State| state.active && !state.stirred;
        let _ = self.stopping.wait_timeout_while(state, time, waits);
    }

    /// Asks the route to look again at once: an alias verifies anew, and
    /// an owner finds out what it is now.
    pub(super) fn stir(&self) {
        self.state().stirred = true;
        self.stopping.notify_all();
        self.route.wake();
    }

    pub(super) fn stirred(&self) -> bool {
        self.state().stirred
    }

    /// A lost link is said first: whatever the machine was, it cannot be
    /// reached just now.
    pub(super) fn standing(&self) -> Standing {
        match self.route.down() {
            Some(LinkFailure::Lost(Lost::NotTrusted | Lost::KeyChanged)) => Standing::NotTrusted,
            Some(LinkFailure::Lost(lost)) => Standing::CannotReach(lost.reason()),
            Some(LinkFailure::Refused(refused)) => Standing::CannotReach(refused.reason()),
            None => self.state().standing.clone(),
        }
    }

    /// How the route stands from now on, said once when it changes.
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
            Standing::Alias(key) => self.log.line(&format!("alias machine={}", key.as_str())),
            _ => {}
        }
        if let Some(keeper) = self.keeper() {
            keeper.changed();
        }
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

    /// Ends the route: its link is closed, and no master is left when
    /// this returns. Whatever ran over it is its machine's to end.
    pub(super) fn stop(&self) {
        let ran = std::mem::replace(&mut self.state().active, false);
        if ran {
            self.log.line("stopped");
        }
        self.stopping.notify_all();
        self.route.stop();
    }
}
