//! One gateway as Agent HUD knows it: its connection, its live sessions and
//! every request waiting in them. `apply` takes what happened and returns
//! what to do about it; nothing here reads a socket or a clock.
//!
//! A card is never left for a request that is over. It closes when Hermes
//! says the request settled, when its turn ends, when its known deadline
//! passes, when an answer to it comes back as already resolved or stale,
//! when a newer snapshot no longer lists it, when its session leaves the
//! live list, and when the gateway restarts.
use super::ask::Pending;
use super::connection::{identify, Connection, Denied, Failure, Identified, Unsupported};
use super::effect::{Closed, Effect, Input, Run, Status};
use super::frame::{answered, Answered, Cursor, Refusal};
use super::identity::{GatewayId, Profile, SessionRef};
use serde_json::Value;
use std::collections::BTreeMap;

/// How often to list the sessions while any of them starts, works or waits,
/// or a card shows.
pub const BUSY_POLL_MS: i64 = 1_000;
/// How often to list them when nothing seems to happen: Hermes's hints and
/// the hooks can be missed, and this is what finds a turn they did not tell.
pub const IDLE_POLL_MS: i64 = 10_000;

#[derive(Debug)]
pub(super) struct Session {
    pub(super) phase: Status,
    pub(super) cursor: Option<Cursor>,
    /// The epochs its stream had before this one. What is read from one of
    /// them arrives late and is dropped.
    pub(super) left: Vec<String>,
    /// The newest turn seen under way, and the newest seen ended.
    pub(super) active: Option<u64>,
    pub(super) ended: Option<u64>,
    pub(super) subscription: Option<String>,
    /// Where the stream was when a snapshot was last asked for.
    pub(super) snapshot_at: Option<Cursor>,
    /// Where a reading went on from without having read what came before.
    pub(super) skipped: Option<Cursor>,
    /// Requests known to be over, each with its turn where it names one,
    /// whether or not a card ever showed: a reading from before the end
    /// does not show one again. A turn that ended takes its entries.
    pub(super) retired: BTreeMap<String, Option<u64>>,
}

#[derive(Debug)]
pub(super) struct Open {
    pub(super) pending: Pending,
    /// When its answer was sent, until Hermes says what became of it.
    pub(super) sent_ms: Option<i64>,
    /// Where in the session's stream it was last known to wait.
    pub(super) seen: Option<Cursor>,
    /// How long Hermes said it waits.
    pub(super) wait_ms: Option<i64>,
}

#[derive(Debug)]
pub struct Gateway {
    pub(super) id: GatewayId,
    pub(super) connection: Connection,
    /// The last instance that was ready; another one means a restart.
    pub(super) instance: Option<String>,
    /// Counts the instances: what an earlier one answers late is dropped.
    pub(super) run: Run,
    pub(super) sessions: BTreeMap<(Profile, String), Session>,
    pub(super) open: BTreeMap<String, Open>,
}

impl Gateway {
    pub fn new(id: GatewayId) -> Self {
        Self {
            id,
            connection: Connection::Disconnected,
            instance: None,
            run: Run(0),
            sessions: BTreeMap::new(),
            open: BTreeMap::new(),
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
        self.open.get(id).map(|open| &open.pending)
    }
    /// Every request waiting, in the order of their ids.
    pub fn waiting(&self) -> impl Iterator<Item = &Pending> {
        self.open.values().map(|open| &open.pending)
    }
    /// The run to give back with whatever is read from a socket from now on.
    pub fn run(&self) -> Run {
        self.run
    }
    /// The earliest known deadline of a card.
    pub fn next_deadline(&self) -> Option<i64> {
        self.waiting().filter_map(|p| p.deadline_ms).min()
    }
    /// When to send the next `Tick`, for the one timer that waits: the
    /// earliest deadline or recheck.
    pub fn next_wake(&self) -> Option<i64> {
        let rechecks = self.waiting().filter_map(|p| p.recheck_ms);
        self.next_deadline().into_iter().chain(rechecks).min()
    }
    /// How long to wait before listing the sessions again.
    pub fn poll_ms(&self) -> i64 {
        let busy = |session: &Session| session.phase != Status::Idle;
        if !self.open.is_empty() || self.sessions.values().any(busy) {
            BUSY_POLL_MS
        } else {
            IDLE_POLL_MS
        }
    }
    pub fn session(&self, profile: &Profile, id: &str) -> SessionRef {
        SessionRef {
            gateway: self.id.clone(),
            profile: profile.clone(),
            id: id.to_owned(),
        }
    }
    /// Every live session with what it is doing: waiting while a request of
    /// its own waits here, else what the gateway last said.
    pub fn statuses(&self) -> Vec<(SessionRef, Status)> {
        self.sessions
            .iter()
            .map(|((profile, id), session)| {
                let at = self.session(profile, id);
                let asked = self.waiting().any(|pending| pending.session == at);
                (
                    at,
                    if asked {
                        Status::Waiting
                    } else {
                        session.phase
                    },
                )
            })
            .collect()
    }
    /// Where the session's stream was last read to.
    pub fn cursor(&self, session: &SessionRef) -> Option<&Cursor> {
        self.entry(session)?.cursor.as_ref()
    }
    /// The subscription to detach once the session's answer is sent.
    pub fn subscription(&self, session: &SessionRef) -> Option<&str> {
        self.entry(session)?.subscription.as_deref()
    }
    fn entry(&self, session: &SessionRef) -> Option<&Session> {
        self.sessions
            .get(&(session.profile.clone(), session.id.clone()))
    }

    /// Takes what happened at `now_ms` and returns what to do, in order. A
    /// request that opens and closes within one reading is never shown, and
    /// what was read in an earlier run of the gateway is dropped.
    pub fn apply(&mut self, input: Input<'_>, now_ms: i64) -> Vec<Effect> {
        let mut effects = Vec::new();
        let current = self.run;
        match input {
            Input::Frame { run, .. }
            | Input::Listed { run, .. }
            | Input::Replayed { run, .. }
            | Input::Resumed { run, .. }
            | Input::Responded { run, .. }
                if run != current => {}
            Input::Dialing => self.connect(Connection::Connecting, &mut effects),
            Input::Identified { answer, home } => self.identified(answer, home, &mut effects),
            Input::Denied(Denied::Failed(failure)) | Input::Lost(failure) => {
                self.lost(failure, &mut effects)
            }
            Input::Denied(Denied::Unsupported(why)) => self.unsupported(why, &mut effects),
            Input::Frame { profile, frame, .. } => self.frame(profile, frame, now_ms, &mut effects),
            Input::Listed {
                profile, result, ..
            } => self.listed(profile, result, &mut effects),
            Input::Replayed {
                profile,
                session,
                result,
                ..
            } => self.replayed(profile, session, result, now_ms, &mut effects),
            Input::Resumed {
                profile, result, ..
            } => self.resumed(profile, result, now_ms, &mut effects),
            Input::Sent { request } => {
                if let Some(open) = self.open.get_mut(request) {
                    open.sent_ms = Some(now_ms);
                }
            }
            Input::Responded { request, reply, .. } => self.responded(request, reply, &mut effects),
            Input::Tick => self.tick(now_ms, &mut effects),
        }
        net(effects)
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
    /// stay, each with the deadline it already had; an answer on its way
    /// may not have arrived, so its card is shown again.
    fn lost(&mut self, failure: Failure, effects: &mut Vec<Effect>) {
        for session in self.sessions.values_mut() {
            session.subscription = None;
        }
        for open in self.open.values_mut() {
            if open.sent_ms.take().is_some() {
                effects.push(Effect::Unanswered {
                    pending: open.pending.clone(),
                    why: None,
                });
            }
        }
        self.connect(Connection::Failed(failure), effects);
    }

    fn responded(&mut self, request: &str, reply: &Value, effects: &mut Vec<Effect>) {
        let why = match answered(reply) {
            Answered::Resolved => Closed::Answered,
            Answered::AlreadyResolved => Closed::AlreadyResolved,
            Answered::Refused(Refusal::StaleGeneration) => Closed::StaleGeneration,
            Answered::Refused(refusal) => {
                if let Some(open) = self.open.get_mut(request) {
                    open.sent_ms = None;
                    effects.push(Effect::Unanswered {
                        pending: open.pending.clone(),
                        why: Some(refusal),
                    });
                }
                return;
            }
        };
        self.close(request, why, effects);
    }
}

/// Drops a card that opens and closes within the same reading.
fn net(effects: Vec<Effect>) -> Vec<Effect> {
    let opened = |id: &str, effect: &Effect| matches!(effect, Effect::Open(p) if p.id() == id);
    let closed =
        |id: &str, effect: &Effect| matches!(effect, Effect::Close { id: c, .. } if c == id);
    let mut kept: Vec<Effect> = Vec::with_capacity(effects.len());
    for effect in effects {
        if let Effect::Close { id, .. } = &effect {
            if let Some(index) = kept.iter().rposition(|earlier| opened(id, earlier)) {
                if !kept[index..].iter().any(|later| closed(id, later)) {
                    kept.remove(index);
                    continue;
                }
            }
        }
        kept.push(effect);
    }
    kept
}
