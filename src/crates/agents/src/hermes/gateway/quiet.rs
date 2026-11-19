//! Sessions nothing has changed in for a long while (ticket 67). Hermes
//! can list a session as working for days: a turn that was cut off, and
//! that nothing ever ended. Its list is all there is to go by, so the
//! session's row says what Hermes says. But a session that hurries the
//! polls for as long as it is listed costs its gateway a poll every
//! second for those days, and a far machine a history read every minute.
//!
//! So the model keeps, for each listed session, what it was last listed
//! as, where its stream was last seen (its epoch and latest event), and
//! when either last changed. A session neither has changed in for
//! `QUIET_MS` is quiet. A quiet session does not hurry the polls
//! (`poll_ms`, `lively`), and is not at work for a far machine's
//! history (`working`). It moves again the moment a poll lists it as
//! something else or finds its stream somewhere else, and counts at
//! once.
//!
//! Nothing else changes for it. A poll reads a quiet session exactly as
//! it read it before, so a request asked in it opens its card at the
//! next poll, and a card that shows always hurries the polls. A read
//! that fails says nothing of where a stream is, so it is no movement:
//! a session whose stream cannot be read goes quiet too, by the list
//! alone.
use super::effect::Status;
use super::identity::SessionRef;
use super::machine::Machine;
use super::poll::Poll;
use super::state::{Gateway, BUSY_POLL_MS, IDLE_POLL_MS};
use serde_json::Value;
use std::collections::BTreeMap;

/// How long nothing may change in a listed session before it is quiet:
/// thirty minutes. It is Hermes's own limit for a turn in which nothing
/// happens at all (`agent.gateway_timeout`, 1800 s): past it Hermes
/// takes the turn for hung and ends it, which moves the stream. It is
/// also the longest one call of a model may take (`HERMES_API_TIMEOUT`,
/// 1800 s), and three times the longest a terminal command may run in
/// a turn before Hermes moves it to the background (600 s). A tool that
/// runs silent for longer still is kept alive by Hermes without a word
/// on the stream; its session goes quiet here, which slows the polls
/// and hides nothing.
pub const QUIET_MS: i64 = 30 * 60 * 1_000;

/// Where a stream is: its epoch and its latest event.
pub(super) type Place = (String, u64);

/// One listed session as it was last seen.
#[derive(Debug)]
struct Seen {
    status: Status,
    /// None until a read of its stream is answered. Learning it for
    /// the first time is no change.
    place: Option<Place>,
    /// When its status or its place last changed, or it was first listed.
    since_ms: i64,
}

/// What the model keeps of each listed session to say whether it is
/// quiet.
#[derive(Debug)]
pub(super) struct Still {
    after_ms: i64,
    seen: BTreeMap<SessionRef, Seen>,
}

impl Default for Still {
    fn default() -> Self {
        Self {
            after_ms: QUIET_MS,
            seen: BTreeMap::new(),
        }
    }
}

impl Still {
    /// The gateway is another one: every session is seen anew.
    pub(super) fn forget(&mut self) {
        self.seen.clear();
    }

    /// A reading that counts found the stream of `at` at `place`.
    fn placed(&mut self, at: &SessionRef, place: Option<&Place>, now_ms: i64) {
        let (Some(seen), Some(place)) = (self.seen.get_mut(at), place) else {
            return;
        };
        // The first place known of a stream is where it was: no move.
        match &seen.place {
            Some(known) if known == place => return,
            Some(_) => seen.since_ms = now_ms,
            None => {}
        }
        seen.place = Some(place.clone());
    }
}

impl Poll {
    /// Notes where an answer to `session.events.since` says the stream
    /// of `at` is. An answer that does not say leaves what is known.
    pub(super) fn placed(&mut self, at: &SessionRef, result: &Value) {
        let epoch = crate::text(&result["replay_epoch"]).or_else(|| crate::text(&result["epoch"]));
        let latest = (result["last_sequence"].as_u64()).or(result["latest_seq"].as_u64());
        if let (Some(epoch), Some(latest)) = (epoch, latest) {
            self.places.insert(at.clone(), (epoch.to_owned(), latest));
        }
    }
}

impl Gateway {
    /// Sets how long nothing may change in a session before it is quiet.
    /// Tests shorten it.
    pub fn quiet_after(&mut self, ms: i64) {
        self.still.after_ms = ms;
    }

    /// Whether nothing has changed in `at` for the quiet time, as far as
    /// the polls have seen.
    pub(super) fn quiet(&self, at: &SessionRef) -> bool {
        let seen = self.still.seen.get(at);
        seen.is_some_and(|seen| self.now_ms - seen.since_ms >= self.still.after_ms)
    }

    /// How long to wait before the next poll: the short wait while a
    /// card shows, or a listed session starts, works or waits and is
    /// not quiet.
    pub fn poll_ms(&self) -> i64 {
        let busy =
            |(at, status): (&SessionRef, &Status)| *status != Status::Idle && !self.quiet(at);
        match !self.cards.is_empty() || self.listed.iter().any(busy) {
            true => BUSY_POLL_MS,
            false => IDLE_POLL_MS,
        }
    }

    /// Whether the connection keeps its close watch on the gateway: its
    /// quicker pace, for a turn that can begin in any listed session
    /// with nothing but a poll to tell of it. On this machine, while
    /// any session is listed. On a far machine, where every poll is
    /// calls over the link and work for a gateway that serves others,
    /// while a card shows or a listed session is not quiet: a gateway
    /// lists its old chats for as long as it runs, and one where
    /// nothing has moved for the quiet time is asked at the model's own
    /// pace until something does.
    pub fn lively(&self) -> bool {
        if !matches!(self.id.machine(), Machine::Far(_)) {
            return !self.listed.is_empty();
        }
        !self.cards.is_empty() || self.listed.keys().any(|at| !self.quiet(at))
    }

    /// Takes what a poll that counts says of each session it listed,
    /// after its list replaced the one before: a session listed anew, or
    /// as something else, or whose stream is somewhere else, has moved
    /// now.
    pub(super) fn stilled(&mut self, poll: &Poll, now_ms: i64) {
        for (at, look) in &poll.sessions {
            match self.still.seen.get_mut(at) {
                Some(seen) if seen.status == look.status => {}
                Some(seen) => (seen.status, seen.since_ms) = (look.status, now_ms),
                None => {
                    let (status, place, since_ms) = (look.status, None, now_ms);
                    let seen = Seen {
                        status,
                        place,
                        since_ms,
                    };
                    self.still.seen.insert(at.clone(), seen);
                }
            }
            self.still.placed(at, poll.places.get(at), now_ms);
        }
        let listed = &self.listed;
        self.still.seen.retain(|at, _| listed.contains_key(at));
    }

    /// The same of a reading of one session apart, which lists nothing:
    /// where its stream is.
    pub(super) fn stilled_apart(&mut self, poll: &Poll, at: &SessionRef, now_ms: i64) {
        self.still.placed(at, poll.places.get(at), now_ms);
    }
}
