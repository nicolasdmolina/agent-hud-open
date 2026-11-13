//! Hermes's native channel (`agents::hermes::GATEWAY`): the local gateway
//! its desktop app talks to. This is the half with the sockets. It finds
//! the gateway's control socket in a Hermes home, gets a ticket as the
//! same user, opens the WebSocket, and runs the polls that
//! `agents::hermes::gateway` begins. What that model says to do is done
//! here and nothing else is decided: a card opens, changes or closes in
//! core's requests, and an answer given on a card goes back on a socket of
//! its own.
//!
//! It reads Hermes and never writes under a Hermes home. It takes no part
//! in collection: it never holds the collection gate, and every wait on
//! the gateway, writes included, happens with nothing locked. An answer
//! never leaves once the observer stopped or its card closed: a write is
//! checked and marked under the model's lock, and stopping or closing cuts
//! the socket of a write that is under way (`visit.rs`).
use crate::hooks::Wake;
use agent_hud_agents::hermes::{
    gateway::{Connection, Gateway, GatewayId, Input, Instance},
    AGENT,
};
use agent_hud_core::requests::{Requests, Ticket as Waiter};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
    },
    thread,
    time::Duration,
};

mod answer;
mod control;
mod effects;
mod link;
mod log;
mod pace;
mod polls;
mod socket;
mod visit;
mod wire;

pub(crate) use pace::Pace;

/// The one gateway phase 1 follows: the one on this machine.
const LOCAL: &str = "local";

pub struct Observer {
    shared: Arc<Shared>,
}

pub(super) struct Shared {
    /// The Hermes home whose gateway is followed.
    home: PathBuf,
    requests: Requests,
    wake: Wake,
    log: log::Log,
    pace: Pace,
    active: AtomicBool,
    /// A poll is wanted before the timer says so, however many asked.
    wanted: AtomicBool,
    model: Mutex<Model>,
    /// Ends a pause early when the observer stops.
    stopping: (Mutex<()>, Condvar),
}

/// The gateway as the model knows it, and what core holds for its cards.
/// Locked only to change it, never across a wait on the network.
pub(super) struct Model {
    gateway: Gateway,
    /// Where the answer to each card arrives from core.
    waiters: HashMap<String, Waiter>,
    /// When each card first opened, which its view keeps.
    opened: HashMap<String, i64>,
    /// The cards answered here whose answer Hermes has yet to settle, and
    /// whether each is a set of questions.
    answered: HashMap<String, bool>,
    /// Every short socket that is open now (`visit.rs`), so that stopping,
    /// or closing a card, can cut the ones that are writing.
    visits: HashMap<u64, visit::Held>,
    /// Counts the visits begun, which numbers them.
    visited: u64,
    stopped: bool,
}

impl Model {
    /// Whether an answer to `request` may still go to `instance`: the
    /// observer has not stopped, the request still has its card, and the
    /// gateway that is ready is still the instance that asked. Anything
    /// else and the answer is for nobody.
    pub(super) fn current(&self, request: &str, instance: &Instance) -> bool {
        let same = matches!(
            self.gateway.connection(),
            Connection::Ready(ready) if ready.id == instance.id
        );
        !self.stopped && same && self.gateway.pending(request).is_some()
    }
}

/// The Hermes home on this machine: `HERMES_HOME`, or `~/.hermes`.
pub fn home() -> Result<PathBuf, String> {
    let user = crate::os::home_dir()?;
    let environment = std::env::vars().collect();
    let sources = AGENT.sources.on(crate::os::OS).value().copied();
    sources
        .unwrap_or_default()
        .iter()
        .flat_map(|source| crate::os::resolve(&user, &environment, &source.location))
        .next()
        .ok_or_else(|| "Hermes has no home on this system".to_owned())
}

impl Observer {
    /// Follows the gateway of this machine's Hermes home, logging into `data`.
    pub fn start(data: &Path, requests: Requests, wake: Wake) -> Result<Self, String> {
        Self::start_at(home()?, data, requests, wake, Pace::default())
    }

    pub(crate) fn start_at(
        home: PathBuf,
        data: &Path,
        requests: Requests,
        wake: Wake,
        pace: Pace,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            home,
            requests,
            wake,
            log: log::Log::new(data),
            pace,
            active: AtomicBool::new(true),
            wanted: AtomicBool::new(false),
            model: Mutex::new(Model {
                gateway: Gateway::new(GatewayId::new(LOCAL)),
                waiters: HashMap::new(),
                opened: HashMap::new(),
                answered: HashMap::new(),
                visits: HashMap::new(),
                visited: 0,
                stopped: false,
            }),
            stopping: (Mutex::new(()), Condvar::new()),
        });
        let failed = |_| "Hermes's gateway could not be followed".to_owned();
        let owned = shared.clone();
        thread::Builder::new()
            .name("Hermes gateway".into())
            .spawn(move || link::work(owned))
            .map_err(failed)?;
        let owned = shared.clone();
        if let Err(error) = thread::Builder::new()
            .name("Hermes answers".into())
            .spawn(move || answer::watch(owned))
        {
            shared.stop();
            return Err(failed(error));
        }
        shared.log.line("started");
        Ok(Self { shared })
    }

    /// A Hermes hook reported a turn started, waiting or ended: poll now.
    pub fn nudge(&self) {
        self.shared.want_poll();
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Shared {
    pub(super) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Gives the model what happened and does what it says. Nothing happens
    /// once the observer stopped: a thread still out on the network comes
    /// back to a model that no longer takes anything.
    pub(super) fn apply(&self, input: Input<'_>) {
        let mut model = self.model();
        if model.stopped {
            return;
        }
        self.take(&mut model, input);
    }

    /// The model, already locked, takes what happened. What it says is
    /// done, and core is told what the sessions are doing whether or not
    /// anything changed: every input goes through here.
    pub(super) fn take(&self, model: &mut Model, input: Input<'_>) {
        let now = crate::hooks::now_ms();
        let effects = model.gateway.apply(input, now);
        self.carry(model, effects, now);
        self.tell(Some(&model.gateway), now);
    }

    /// The instance to answer `request` on: the one that is ready, while
    /// the observer runs and the request still has its card. A gateway
    /// that restarted since the card was answered closed the card, and
    /// its new instance never asked.
    pub(super) fn instance_of(&self, request: &str) -> Option<Instance> {
        let model = self.model();
        if model.stopped {
            return None;
        }
        model.gateway.pending(request)?;
        match model.gateway.connection() {
            Connection::Ready(instance) => Some(instance.clone()),
            _ => None,
        }
    }

    /// Whether an answer to `request` may still go to `instance`, for the
    /// tests that ask from outside a visit.
    #[cfg(test)]
    pub(super) fn current(&self, request: &str, instance: &Instance) -> bool {
        self.model().current(request, instance)
    }

    pub(super) fn want_poll(&self) {
        self.wanted.store(true, Ordering::Release);
    }

    pub(super) fn is_wanted(&self) -> bool {
        self.wanted.load(Ordering::Acquire)
    }

    pub(super) fn take_wanted(&self) -> bool {
        self.wanted.swap(false, Ordering::AcqRel)
    }

    /// Waits `time`, or less when the observer stops.
    pub(super) fn pause(&self, time: Duration) {
        let (lock, stopping) = &self.stopping;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if self.active() {
            let _ = stopping.wait_timeout(guard, time);
        }
    }

    /// Withdraws every card and takes nothing more. The threads end on
    /// their own, the one on the network when its wait does.
    fn stop(&self) {
        self.active.store(false, Ordering::Release);
        {
            // Held while waking, so a pause that just saw it active is
            // already waiting and hears this.
            let (lock, stopping) = &self.stopping;
            let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
            stopping.notify_all();
        }
        let mut model = self.model();
        if model.stopped {
            return;
        }
        model.stopped = true;
        // Whatever is being written right now is cut before this returns:
        // nothing leaves once the observer has stopped.
        model.cut_writing(|_| true);
        let ids: Vec<String> = model.gateway.waiting().map(|card| card.id()).collect();
        for id in ids {
            self.requests.settle_native(&id);
        }
        model.waiters.clear();
        model.opened.clear();
        model.answered.clear();
        drop(model);
        self.tell(None, crate::hooks::now_ms());
        self.log.line("stopped");
    }
}

#[cfg(all(test, unix))]
mod tests_unix;
