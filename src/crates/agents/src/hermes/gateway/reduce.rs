//! How the gateway's state follows what Hermes says as it happens: each
//! event of a session's stream, and each request the gateway makes of its
//! viewers. `readings` holds what is read on request.
use super::ask::Ask;
use super::cards::Heard;
use super::effect::{Closed, Effect, Status};
use super::frame::{frame, Event, Frame};
use super::identity::Profile;
use super::state::Gateway;
use serde_json::Value;

impl Gateway {
    pub(super) fn frame(
        &mut self,
        profile: &Profile,
        value: &Value,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        match frame(value) {
            Frame::Event(event) => self.event(profile, &event, Heard::Live, now_ms, effects),
            Frame::Request { id, method, params } => {
                let session = params["session_id"].as_str().unwrap_or("");
                if let (false, Some(ask)) = (session.is_empty(), Ask::batch(id, method, params)) {
                    // A request is not an event and has no place in the
                    // stream of its own: it waits from where the stream is.
                    let seen = self.entry_mut(profile, session).cursor.clone();
                    let at = self.session(profile, session);
                    self.show(at, ask, seen, Heard::Live, None, now_ms, effects);
                }
            }
            Frame::Reply { .. } | Frame::Other => {}
        }
    }

    pub(super) fn event(
        &mut self,
        profile: &Profile,
        event: &Event<'_>,
        heard: Heard,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        if event.session.is_empty() {
            // Something about the sessions changed: a hint to list them
            // again, not a promise that every change is told.
            if matches!(event.kind, "sessions.changed" | "platforms.changed") {
                effects.push(Effect::List);
            }
            return;
        }
        let at = self.session(profile, event.session);
        let session = self.entry_mut(profile, event.session);
        let mut moved = false;
        if let Some(seen) = &event.at {
            match &mut session.cursor {
                Some(cursor) if cursor.epoch == seen.epoch => {
                    // Read already, through the socket or an earlier replay.
                    if seen.seq <= cursor.seq {
                        return;
                    }
                    cursor.seq = seen.seq;
                }
                // From a stream the session had before: it arrives late.
                Some(_) if session.left.contains(&seen.epoch) => return,
                Some(cursor) => {
                    let before = std::mem::replace(cursor, seen.clone());
                    session.left.push(before.epoch);
                    moved = true;
                }
                cursor => *cursor = Some(seen.clone()),
            }
        }
        // An event of a turn says that turn is under way, or was.
        if let Some(turn) = event.generation {
            session.active = session.active.max(Some(turn));
        }
        let newest = event
            .generation
            .is_none_or(|turn| session.active == Some(turn));
        match event.kind {
            "clarify.request" | "approval.request" => {
                if let Some((ask, wait_ms)) = Ask::prompt(event.payload) {
                    let seen = event.at.clone();
                    self.show(at.clone(), ask, seen, heard, wait_ms, now_ms, effects);
                }
            }
            "clarify.settled" | "approval.settled" => {
                if let Some(prompt) = crate::text(&event.payload["prompt_id"]) {
                    let turn = event.payload["execution_generation"]
                        .as_u64()
                        .or(event.generation);
                    self.end(&at, &at.request_id(prompt), Closed::Settled, turn, effects);
                }
            }
            "request.cancel" => {
                // Hermes's reasons are an open set; any of them ends the request.
                let why = match event.payload["reason"].as_str().unwrap_or("") {
                    "resolved" => Closed::Settled,
                    "timeout" => Closed::TimedOut,
                    "interrupted" => Closed::Interrupted,
                    other => Closed::Withdrawn(other.to_owned()),
                };
                if let Some(request) = crate::text(&event.payload["id"]) {
                    self.end(&at, &at.request_id(request), why, None, effects);
                }
            }
            "message.start" if newest => session.phase = Status::Working,
            "message.complete" => {
                // Only the turn that ended, and those before it: a
                // completion read late does not make a newer turn idle,
                // take its card, or forget what it settled. A set of
                // questions names no turn; Hermes withdraws it.
                let Some(ended) = event.generation else {
                    return;
                };
                if newest {
                    session.phase = Status::Idle;
                }
                session.ended = session.ended.max(Some(ended));
                let why = match event.payload["status"].as_str() {
                    Some("interrupted") => Closed::Interrupted,
                    _ => Closed::TurnEnded,
                };
                for id in self.asked(&at, |ask| ask.generation().is_some_and(|g| g <= ended)) {
                    self.close(&id, why.clone(), effects);
                }
                // What ended with the turn needs no memory of its own: a
                // request of a turn that ended is never shown.
                self.entry_mut(profile, event.session)
                    .retired
                    .retain(|_, turn| turn.is_none_or(|turn| turn > ended));
            }
            _ => {}
        }
        // The stream is a new one. What was last seen in the old one can
        // no longer be settled by events, and events may have been missed
        // between the two: a snapshot says what waits now.
        let gap = event.at.as_ref().is_some_and(|seen| seen.seq > 1);
        if moved && heard == Heard::Live && (gap || self.strays(&at)) {
            Self::resume(&at, effects);
        }
    }
}
