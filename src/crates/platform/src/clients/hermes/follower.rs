//! One follower: the connection to one gateway, its model, and the two
//! threads that keep them (`link.rs`, `answer.rs`). A machine can run
//! several gateways, and each has a follower of its own (`followers.rs`).
//!
//! A follower is made for one gateway and is that gateway's for as long
//! as it lives: the id it was given names its cards and is the name it
//! tells core everything under, and neither changes. It ends when client
//! hooks turn off, or when a look at the whole machine shows that its
//! gateway is gone (`discovery.rs`); it never ends for being out of
//! reach. Ending is one step, taken once: what it was writing is cut,
//! its cards are closed or withdrawn, and core is told its reader has
//! ended, before any follower that takes its place is made.
use super::{log, out, route::Route, signal::Signal, visit, Pace};
use crate::hooks::Wake;
use agent_hud_agents::hermes::{
    gateway::{Connection, Gateway, GatewayId, Input, Instance, Profile},
    AGENT,
};
use agent_hud_core::requests::{Requests, Ticket as Waiter};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
    },
    time::Duration,
};

pub(super) struct Shared {
    /// The way to the gateway that is followed.
    pub(super) route: Arc<dyn Route>,
    pub(super) requests: Requests,
    pub(super) wake: Wake,
    pub(super) log: log::Log,
    pub(super) pace: Pace,
    pub(super) active: AtomicBool,
    /// A poll is wanted before the timer says so, however many asked.
    pub(super) wanted: AtomicBool,
    pub(super) model: Mutex<Model>,
    /// Ends a pause early when the follower ends.
    pub(super) stopping: (Mutex<()>, Condvar),
    /// The name everything is told to core under: the gateway's id, as it
    /// is written. Core keeps each reader's readings apart (ticket 24).
    pub(super) reader: String,
    /// The profile whose home is asked, for who is there and for tickets.
    /// It is the gateway's anchor until that profile leaves the gateway.
    pub(super) asked: Mutex<Profile>,
    /// The answer this follower was made from, where a look at the machine
    /// found its gateway: its first identify takes it and asks nothing.
    pub(super) born: Mutex<Option<Value>>,
    /// The instance its own identify found at its home in place of the one
    /// it follows. What that means is not its to say: the gateway may have
    /// restarted, or the profile may be another gateway's now.
    pub(super) found: Mutex<Option<String>>,
    /// Asks for a look at every gateway of the machine.
    pub(super) look: Arc<Signal>,
    /// Every reader that was ended in core, for the tests that count them.
    #[cfg(test)]
    pub(super) ends: Arc<Mutex<Vec<String>>>,
}

/// The gateway as the model knows it, and what core holds for its cards.
/// Locked only to change it, never across a wait on the network.
pub(super) struct Model {
    pub(super) gateway: Gateway,
    /// Where the answer to each card arrives from core.
    pub(super) waiters: HashMap<String, Waiter>,
    /// When each card first opened, which its view keeps.
    pub(super) opened: HashMap<String, i64>,
    /// The answers given here that are out, or on their way out, and whose
    /// receipt is still to give (`answer.rs`).
    pub(super) answered: HashMap<String, out::Out>,
    /// Every short socket that is open now (`visit.rs`), so that stopping,
    /// or closing a card, can cut the ones that are writing.
    pub(super) visits: HashMap<u64, visit::Held>,
    /// Counts the visits begun, which numbers them.
    pub(super) visited: u64,
    pub(super) stopped: bool,
}

impl Model {
    pub(super) fn new(id: GatewayId) -> Self {
        Self {
            gateway: Gateway::new(id),
            waiters: HashMap::new(),
            opened: HashMap::new(),
            answered: HashMap::new(),
            visits: HashMap::new(),
            visited: 0,
            stopped: false,
        }
    }

    /// Whether an answer to `request` may still go to `instance`: the
    /// follower has not ended, the request still has its card, and the
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

/// Why a follower ends.
pub(super) enum End {
    /// Client hooks turned off: every card is withdrawn without a word.
    Off,
    /// Its home answers as another instance: its cards close as a restart.
    Restarted,
    /// Nothing is left for it to follow, or its id is another gateway's
    /// now: whatever it still shows closes as retired.
    Replaced,
}

/// What a route waits on beside its own work: the follower ending.
impl super::route::Stop for Shared {
    fn wait(&self, time: Duration) -> bool {
        self.pause(time);
        !self.active()
    }
}

impl Shared {
    pub(super) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The profile whose home is asked.
    pub(super) fn asked(&self) -> Profile {
        let asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        asked.clone()
    }

    /// Gives the model what happened and does what it says. Nothing happens
    /// once the follower ended: a thread still out on the network comes
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
        self.tell(&model.gateway, now);
    }

    /// The instance to answer `request` on: the one that is ready, while
    /// the follower runs and the request still has its card. A gateway
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

    /// Waits `time`, or less when the follower ends.
    pub(super) fn pause(&self, time: Duration) {
        let (lock, stopping) = &self.stopping;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if self.active() {
            let _ = stopping.wait_timeout(guard, time);
        }
    }

    /// Ends the follower as turning client hooks off does, for the tests
    /// that stop one.
    #[cfg(test)]
    pub(super) fn stop(&self) {
        self.end(End::Off);
    }

    /// Ends a pause that is under way, so that a gateway that is back is
    /// asked now and not when the wait would have ended.
    pub(super) fn stir(&self) {
        let (lock, stopping) = &self.stopping;
        let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
        stopping.notify_all();
    }

    /// Ends the follower, once: a second call does nothing, whenever it
    /// comes, so an end that arrives late can clear nothing a follower
    /// made since has told core. Its cards go as `why` says and it takes
    /// nothing more. The threads end on their own, the one on the network
    /// when its wait does. Core is told last that this reader has ended.
    pub(super) fn end(&self, why: End) {
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
        // nothing leaves once the follower has ended.
        model.cut_writing(|_| true);
        let now = crate::hooks::now_ms();
        let closing = match why {
            End::Off => None,
            End::Restarted => Some(model.gateway.apply(Input::Restarted, now)),
            End::Replaced => {
                let held = model.gateway.holds().clone();
                let shown = model
                    .gateway
                    .waiting()
                    .map(|card| card.session.profile.clone());
                let all = held.into_iter().chain(shown).collect();
                Some(model.gateway.apply(Input::Retired(&all), now))
            }
        };
        match closing {
            // Closed as the model says, so an answer that is out keeps the
            // receipt Hermes's reply gives it (`out.rs`, `answer.rs`).
            Some(effects) => self.carry(&mut model, effects, now),
            None => {
                let ids: Vec<String> = model.gateway.waiting().map(|card| card.id()).collect();
                for id in ids {
                    self.requests.settle_native(&id);
                }
                model.answered.clear();
            }
        }
        model.waiters.clear();
        model.opened.clear();
        drop(model);
        self.requests.end_native_reader(AGENT.id, &self.reader);
        #[cfg(test)]
        (self.ends.lock().unwrap()).push(self.reader.clone());
    }
}
