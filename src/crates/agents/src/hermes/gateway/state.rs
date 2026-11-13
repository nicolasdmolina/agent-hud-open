//! One gateway as Agent HUD knows it: its connection and the cards that
//! show. Nothing here reads a socket or a clock.
//!
//! The cards follow polls and nothing else. Each poll asks Hermes what
//! waits now (`poll.rs`), and `reconcile.rs` opens, changes and closes
//! cards until they match what it said. Between polls the model keeps the
//! cards, the gateway's instance, a count of polls, and when it last took
//! a snapshot of each waiting session. It keeps no place in any stream
//! and no memory of what was over, so there is no order in which two
//! readings can be merged wrongly: a poll is whole, the next one starts
//! after it, and one that is no longer the newest when it comes back is
//! dropped.
use super::ask::{Ask, Pending};
use super::connection::{identify, Connection, Denied, Failure, Identified, Unsupported};
use super::effect::{Closed, Effect, Input, Status};
use super::frame::{answered, Answered, Refusal};
use super::identity::{GatewayId, Profile, SessionRef};
use super::poll::{Poll, Shown};
use serde_json::Value;
use std::collections::BTreeMap;

/// How long to wait between polls while any session starts, works or
/// waits, or a card shows.
pub const BUSY_POLL_MS: i64 = 1_000;
/// How long when nothing seems to happen: Hermes's hints and the hooks can
/// be missed, and this is what finds a turn they did not tell.
pub const IDLE_POLL_MS: i64 = 10_000;
/// The least time between two snapshots of one session while it goes on
/// waiting. A request that no read of the session's events can show (its
/// event dropped, or not reached) is found by the next snapshot, so this is
/// also how long such a request can stay hidden.
pub const SNAPSHOT_EVERY_MS: i64 = 30_000;

/// One request as the model files it: where it waits, and the gateway's
/// own id for it. The gateway is part of every key.
pub(super) type Key = (SessionRef, String);

#[derive(Debug)]
pub(super) struct Card {
    pub(super) pending: Pending,
    /// When its answer was written, until Hermes says what became of it.
    pub(super) sent_ms: Option<i64>,
}

#[derive(Debug)]
pub struct Gateway {
    pub(super) id: GatewayId,
    pub(super) connection: Connection,
    /// The last instance that was ready; another one means a restart.
    pub(super) instance: Option<String>,
    /// Counts the polls begun, and every change a poll begun before it
    /// cannot know of: an answer Hermes took, a restart, a dropped socket.
    pub(super) polls: u64,
    pub(super) cards: BTreeMap<Key, Card>,
    /// What the last poll listed each live session as doing. It is only
    /// reported (`statuses`, `poll_ms`); no poll reads it.
    pub(super) listed: BTreeMap<SessionRef, Status>,
    /// When a snapshot was last taken of each session that waits. Hermes
    /// drops a stream's old events, and a request asked before the oldest
    /// one kept shows in no read. A snapshot finds it, and subscribes to
    /// do so, so one is not taken at every poll: this says when the last
    /// one was. It is the one thing a poll leaves for the next, beside the
    /// cards. It is forgotten when the session stops waiting, when a card
    /// of the session closes, when the socket is lost and when the gateway
    /// restarts, so that the next poll takes a snapshot again.
    pub(super) snapshotted: BTreeMap<SessionRef, i64>,
    /// The latest time `apply` was given.
    pub(super) now_ms: i64,
}

impl Gateway {
    pub fn new(id: GatewayId) -> Self {
        Self {
            id,
            connection: Connection::Disconnected,
            instance: None,
            polls: 0,
            cards: BTreeMap::new(),
            listed: BTreeMap::new(),
            snapshotted: BTreeMap::new(),
            now_ms: 0,
        }
    }
    pub fn id(&self) -> &GatewayId {
        &self.id
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    /// The request with this id, while it waits.
    pub fn pending(&self, id: &str) -> Option<&Pending> {
        self.waiting().find(|pending| pending.id() == id)
    }
    /// Every request waiting, session by session.
    pub fn waiting(&self) -> impl Iterator<Item = &Pending> {
        self.cards.values().map(|card| &card.pending)
    }
    pub fn session(&self, profile: &Profile, id: &str) -> SessionRef {
        SessionRef {
            gateway: self.id.clone(),
            profile: profile.clone(),
            id: id.to_owned(),
        }
    }
    /// What a live session is doing: waiting while a request of its own
    /// shows here, else what the last poll listed. None for a session that
    /// is not live, or not this gateway's.
    pub fn status(&self, session: &SessionRef) -> Option<Status> {
        let listed = *self.listed.get(session)?;
        let asked = self.cards.keys().any(|(at, _)| at == session);
        Some(if asked { Status::Waiting } else { listed })
    }
    /// Every live session with what it is doing.
    pub fn statuses(&self) -> Vec<(SessionRef, Status)> {
        let known = |at: &SessionRef| Some((at.clone(), self.status(at)?));
        self.listed.keys().filter_map(known).collect()
    }
    /// How long to wait before the next poll.
    pub fn poll_ms(&self) -> i64 {
        let busy = |status: &Status| *status != Status::Idle;
        if !self.cards.is_empty() || self.listed.values().any(busy) {
            BUSY_POLL_MS
        } else {
            IDLE_POLL_MS
        }
    }

    /// Begins a poll of the gateway, which is the newest from now on. None
    /// while the gateway is not ready. Run one poll at a time: beginning
    /// another makes the one before it stale.
    pub fn poll(&mut self) -> Option<Poll> {
        let Connection::Ready(instance) = &self.connection else {
            return None;
        };
        self.polls += 1;
        let mut shown: BTreeMap<SessionRef, Shown> = BTreeMap::new();
        for ((session, key), card) in &self.cards {
            let of = shown.entry(session.clone()).or_default();
            match card.pending.ask {
                Ask::Batch(_) => of.sets.push(key.clone()),
                Ask::Question(_) | Ask::Approval(_) => of.prompts.push(key.clone()),
            }
        }
        let served = instance.served.iter().map(|served| served.profile.clone());
        // Counted from the latest time the model was told of.
        let recent = self
            .snapshotted
            .iter()
            .filter(|(_, at)| self.now_ms - **at < SNAPSHOT_EVERY_MS)
            .map(|(session, _)| session.clone())
            .collect();
        Some(Poll::new(
            self.id.clone(),
            self.polls,
            served,
            shown,
            recent,
        ))
    }

    /// Takes what happened at `now_ms` and returns what to do, in order.
    pub fn apply(&mut self, input: Input<'_>, now_ms: i64) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.now_ms = now_ms;
        match input {
            Input::Dialing => self.connect(Connection::Connecting, &mut effects),
            Input::Identified { answer, home } => self.identified(answer, home, &mut effects),
            Input::Denied(Denied::Failed(failure)) | Input::Lost(failure) => {
                self.lost(failure, &mut effects)
            }
            Input::Denied(Denied::Unsupported(why)) => self.unsupported(why, &mut effects),
            Input::Polled(poll) => self.reconcile(poll, now_ms, &mut effects),
            Input::Sent { request } => self.sent(request, now_ms, &mut effects),
            Input::Responded { request, reply } => self.responded(request, reply, &mut effects),
        }
        effects
    }

    fn connect(&mut self, next: Connection, effects: &mut Vec<Effect>) {
        if self.connection != next {
            self.connection = next.clone();
            effects.push(Effect::Connection(next));
        }
    }

    fn identified(&mut self, answer: &Value, home: &str, effects: &mut Vec<Effect>) {
        match identify(answer, home) {
            Identified::Ready(instance) => {
                // Another instance is a restart: nothing asked before it
                // survives, whatever port it came back on.
                if self.instance.as_ref().is_some_and(|id| *id != instance.id) {
                    self.reset(Closed::GatewayRestarted, effects);
                }
                self.instance = Some(instance.id.clone());
                self.connect(Connection::Ready(instance), effects);
            }
            Identified::Wait(_) => self.connect(Connection::Connecting, effects),
            Identified::Unsupported(why) => self.unsupported(why, effects),
            Identified::Failed(failure) => self.lost(failure, effects),
        }
    }

    /// A gateway without the live connection: the hooks report it alone.
    fn unsupported(&mut self, why: Unsupported, effects: &mut Vec<Effect>) {
        self.reset(Closed::Unsupported, effects);
        self.connect(Connection::Unsupported(why), effects);
    }

    /// The socket is gone. What waits keeps waiting in Hermes, so the cards
    /// stay; an answer on its way may not have arrived, so its card is
    /// shown again. A poll under way cannot end.
    fn lost(&mut self, failure: Failure, effects: &mut Vec<Effect>) {
        self.polls += 1;
        // What happened while nothing was read is not known.
        self.snapshotted.clear();
        for card in self.cards.values_mut() {
            if card.sent_ms.take().is_some() {
                effects.push(Effect::Unanswered {
                    pending: card.pending.clone(),
                    why: None,
                });
            }
        }
        self.connect(Connection::Failed(failure), effects);
    }

    /// Closes every card and forgets the sessions: what a poll of the
    /// gateway as it was says no longer counts.
    fn reset(&mut self, why: Closed, effects: &mut Vec<Effect>) {
        self.polls += 1;
        self.listed.clear();
        self.snapshotted.clear();
        for key in self.cards.keys().cloned().collect::<Vec<_>>() {
            self.close(&key, why.clone(), effects);
        }
    }

    pub(super) fn close(&mut self, key: &Key, why: Closed, effects: &mut Vec<Effect>) {
        if let Some(card) = self.cards.remove(key) {
            // What the session waited on is over. If it still waits, it
            // waits on something else, which a snapshot may have to find.
            self.snapshotted.remove(&card.pending.session);
            effects.push(Effect::Close {
                id: card.pending.id(),
                session: card.pending.session,
                why,
            });
        }
    }

    fn key(&self, request: &str) -> Option<Key> {
        let is = |card: &&Card| card.pending.id() == request;
        let pending = &self.cards.values().find(is)?.pending;
        Some((pending.session.clone(), pending.ask.key().to_owned()))
    }

    /// An answer was written. Nothing replies to the answer to a set of
    /// questions, so only a poll tells what became of it.
    fn sent(&mut self, request: &str, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(card) = self.key(request).and_then(|key| self.cards.get_mut(&key)) else {
            return;
        };
        card.sent_ms = Some(now_ms);
        if matches!(card.pending.ask, Ask::Batch(_)) {
            effects.push(Effect::Poll);
        }
    }

    /// Hermes replied to an answer. Resolved, already resolved and a turn
    /// that is over all mean the request is over: the card closes, a poll
    /// begun before the reply is dropped (it may still say the request
    /// waits), and a new one is asked for at once.
    fn responded(&mut self, request: &str, reply: &Value, effects: &mut Vec<Effect>) {
        let Some(key) = self.key(request) else {
            return;
        };
        let why = match answered(reply) {
            Answered::Resolved => Closed::Answered,
            Answered::AlreadyResolved => Closed::AlreadyResolved,
            Answered::Refused(Refusal::StaleGeneration) => Closed::StaleGeneration,
            Answered::Refused(refusal) => {
                if let Some(card) = self.cards.get_mut(&key) {
                    card.sent_ms = None;
                    effects.push(Effect::Unanswered {
                        pending: card.pending.clone(),
                        why: Some(refusal),
                    });
                }
                return;
            }
        };
        self.polls += 1;
        self.close(&key, why, effects);
        effects.push(Effect::Poll);
    }
}
