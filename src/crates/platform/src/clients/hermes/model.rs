//! What one follower keeps behind its lock: the gateway as the pure model
//! knows it, and what core and the short sockets hold for its cards. It
//! also says whether an answer may still go, which is asked under that
//! lock before any socket is opened for one and before every frame.
use super::{out, visit};
use agent_hud_agents::hermes::gateway::{Connection, Gateway, GatewayId, Instance};
use agent_hud_core::requests::Ticket as Waiter;
use std::collections::HashMap;

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
    /// The decision that is out for each card core put away: the wait it
    /// was taken in (core's ticket), by which the card is given back.
    pub(super) taken: HashMap<String, u64>,
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
            taken: HashMap::new(),
            visits: HashMap::new(),
            visited: 0,
            stopped: false,
        }
    }

    /// Whether an answer to `request` may still go to `instance`: the
    /// follower has not ended, the request still has its card, that card
    /// is not marked as out of reach, and the gateway that is ready is
    /// still the instance that asked. Anything else and the answer is
    /// for nobody.
    pub(super) fn current(&self, request: &str, instance: &Instance) -> bool {
        let same = matches!(
            self.gateway.connection(),
            Connection::Ready(ready) if ready.id == instance.id
        );
        let waits = self.gateway.pending(request).is_some() && !self.gateway.marked(request);
        !self.stopped && same && waits
    }

    /// The instance to answer `request` on: the one that is ready, while
    /// the follower runs, the request still has its card and that card is
    /// not marked as out of reach. A gateway that restarted since the card
    /// was answered closed the card, and its new instance never asked.
    /// This is asked before any socket is opened for an answer: nothing
    /// is sent through a connection that is lost.
    pub(super) fn instance_of(&self, request: &str) -> Option<Instance> {
        if self.stopped || self.gateway.marked(request) {
            return None;
        }
        self.gateway.pending(request)?;
        match self.gateway.connection() {
            Connection::Ready(instance) => Some(instance.clone()),
            _ => None,
        }
    }
}
