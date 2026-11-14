//! The followers of one machine's gateways: one for each gateway found
//! on it (`follower.rs`), and the look that finds them (`discovery.rs`).
//!
//! This machine's own gateway, the one that serves the root home, has a
//! follower from the start, as it always had: it asks the root home, and
//! goes on asking where no gateway is there. Every other follower is made
//! when a look finds a gateway that has none, and is named by that
//! gateway's anchor profile, picked then and kept for its life.
//!
//! The set is locked to make a follower and to end one, never across a
//! wait on the network. A follower that ends has ended, its reader in
//! core with it, before the lock is let go and so before any follower
//! that takes its place is made.
use super::{
    decisions, discovery,
    follower::{End, Shared},
    link, log,
    model::Model,
    route::Route,
    signal::Signal,
    Pace,
};
use crate::hooks::Wake;
use agent_hud_agents::hermes::gateway::{GatewayId, Input, Machine, Profile};
use agent_hud_core::requests::Requests;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
    },
    thread,
};

pub(super) struct Followers {
    /// The way to the machine.
    pub(super) route: Arc<dyn Route>,
    /// The machine the route leads to.
    pub(super) machine: Machine,
    /// The label its cards carry: a far machine's, none for this one.
    label: Option<String>,
    requests: Requests,
    wake: Wake,
    pub(super) log: log::Log,
    pub(super) pace: Pace,
    active: AtomicBool,
    /// Asks for a look at every gateway of the machine.
    pub(super) look: Arc<Signal>,
    /// The followers there are.
    set: Mutex<Vec<Arc<Shared>>>,
    /// The profiles two gateways claim, as far as the looks so far know.
    pub(super) contested: Mutex<BTreeSet<Profile>>,
    /// Every reader that was ended in core, for the tests that count them.
    #[cfg(test)]
    pub(super) ends: Arc<Mutex<Vec<String>>>,
}

impl Followers {
    /// Follows the gateways of `machine`, which `route` leads to, logging
    /// into `data`. A far machine is one that was verified: its key names
    /// every session and card of it, and `label` is what its cards say.
    pub(super) fn start(
        route: Arc<dyn Route>,
        (machine, label): (Machine, Option<String>),
        data: &Path,
        requests: Requests,
        wake: Wake,
        pace: Pace,
    ) -> Result<Arc<Self>, String> {
        // The lines of a far machine's followers begin with its key.
        let who = match &machine {
            Machine::This => None,
            Machine::Far(key) => Some(key.as_str().to_owned()),
        };
        let all = Arc::new(Self {
            route,
            machine,
            label,
            requests,
            wake,
            log: log::Log::new(data).of(who.as_deref()),
            pace,
            active: AtomicBool::new(true),
            look: Arc::default(),
            set: Mutex::default(),
            contested: Mutex::default(),
            #[cfg(test)]
            ends: Arc::default(),
        });
        // Where no look goes by the clock, none is waited for.
        if all.pace.discover.is_none() {
            all.look.set_looked();
        }
        // The machine's own gateway is followed from the start, found or
        // not: it is the one that serves the root home.
        let own = GatewayId::new(all.machine.clone(), Profile::Default);
        all.make(&mut all.set(), own, Profile::Default, None)?;
        let owned = all.clone();
        let looking = thread::Builder::new()
            .name("Hermes gateways".into())
            .spawn(move || discovery::work(owned));
        if looking.is_err() {
            all.stop();
            return Err(unfollowed());
        }
        all.log.line("started");
        Ok(all)
    }

    pub(super) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn set(&self) -> MutexGuard<'_, Vec<Arc<Shared>>> {
        self.set.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the set is locked right now, for the test that looks while
    /// a gateway is being asked.
    #[cfg(test)]
    pub(super) fn locked(&self) -> bool {
        self.set.try_lock().is_err()
    }

    /// A Hermes hook reported a turn started, waiting or ended: every
    /// gateway is polled now. A hook does not say which gateway it is of.
    pub(super) fn nudge(&self) {
        for follower in self.set().iter() {
            follower.want_poll();
        }
    }

    /// Ends every follower and makes no more. Each withdraws its cards
    /// and ends its reader; the threads end on their own.
    pub(super) fn stop(&self) {
        let ran = self.active.swap(false, Ordering::AcqRel);
        self.look.wake();
        for follower in self.set().drain(..) {
            follower.end(End::Off);
        }
        if ran {
            self.log.line("stopped");
        }
    }

    /// Makes the follower of the gateway `id`, which asks the home of
    /// `asked`. `born` is the answer a look found its gateway by. None is
    /// made once the observer has stopped.
    pub(super) fn make(
        &self,
        set: &mut Vec<Arc<Shared>>,
        id: GatewayId,
        asked: Profile,
        born: Option<Value>,
    ) -> Result<(), String> {
        if !self.active() {
            return Ok(());
        }
        let reader = id.to_string();
        let own = *id.machine() == Machine::This && *id.anchor() == Profile::Default;
        let mut model = Model::new(id);
        // No card opens for a profile two instances claim, from its first
        // poll on.
        let contested = self
            .contested
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !contested.is_empty() {
            let now = crate::hooks::now_ms();
            model.gateway.apply(Input::Withheld(&contested), now);
        }
        drop(contested);
        let shared = Arc::new(Shared {
            route: self.route.clone(),
            requests: self.requests.clone(),
            wake: self.wake.clone(),
            log: self.log.of((!own).then_some(reader.as_str())),
            pace: self.pace,
            active: AtomicBool::new(true),
            wanted: AtomicBool::new(false),
            model: Mutex::new(model),
            stopping: (Mutex::new(()), Condvar::new()),
            reader,
            label: self.label.clone(),
            asked: Mutex::new(asked),
            born: Mutex::new(born),
            found: Mutex::default(),
            gone: AtomicBool::new(false),
            awaits: AtomicU64::new(0),
            look: self.look.clone(),
            #[cfg(test)]
            ends: self.ends.clone(),
            #[cfg(test)]
            shows: Default::default(),
            #[cfg(test)]
            deaf: AtomicBool::new(false),
        });
        let owned = shared.clone();
        thread::Builder::new()
            .name("Hermes gateway".into())
            .spawn(move || link::work(owned))
            .map_err(|_| unfollowed())?;
        let owned = shared.clone();
        if thread::Builder::new()
            .name("Hermes answers".into())
            .spawn(move || decisions::watch(owned))
            .is_err()
        {
            shared.end(End::Off);
            return Err(unfollowed());
        }
        set.push(shared);
        Ok(())
    }
}

fn unfollowed() -> String {
    "Hermes's gateway could not be followed".to_owned()
}
