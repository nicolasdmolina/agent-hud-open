//! How the gateway's state follows what Hermes says as it happens: each
//! event of a session's stream, and each request the gateway makes of its
//! viewers. `readings` holds what is read on request.
use super::ask::{Ask, Pending};
use super::effect::{Closed, Effect, Status};
use super::frame::{frame, Event, Frame};
use super::identity::{Profile, SessionRef};
use super::state::{Gateway, Open, Session};
use serde_json::Value;
use std::collections::BTreeSet;

impl Gateway {
    pub(super) fn frame(
        &mut self,
        profile: &Profile,
        value: &Value,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        match frame(value) {
            Frame::Event(event) => self.event(profile, &event, now_ms, effects),
            Frame::Request { id, method, params } => {
                let session = params["session_id"].as_str().unwrap_or("");
                if let (false, Some(ask)) = (session.is_empty(), Ask::batch(id, method, params)) {
                    self.entry_mut(profile, session);
                    self.show(self.session(profile, session), ask, None, effects);
                }
            }
            Frame::Reply { .. } | Frame::Other => {}
        }
    }

    pub(super) fn event(
        &mut self,
        profile: &Profile,
        event: &Event<'_>,
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
        if let Some(seen) = &event.at {
            match &mut session.cursor {
                // Read already, through the socket or an earlier replay.
                Some(cursor) if cursor.epoch == seen.epoch && seen.seq <= cursor.seq => return,
                Some(cursor) if cursor.epoch == seen.epoch => cursor.seq = seen.seq,
                cursor => *cursor = Some(seen.clone()),
            }
        }
        match event.kind {
            "clarify.request" | "approval.request" => {
                if let Some((ask, wait_ms)) = Ask::prompt(event.payload) {
                    let deadline = wait_ms.map(|wait| now_ms.saturating_add(wait));
                    self.show(at, ask, deadline, effects);
                }
            }
            "clarify.settled" | "approval.settled" => {
                if let Some(prompt) = crate::text(&event.payload["prompt_id"]) {
                    self.close(&at.request_id(prompt), Closed::Settled, effects);
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
                    self.close(&at.request_id(request), why, effects);
                }
            }
            "message.start" => session.phase = Status::Working,
            "message.complete" => {
                session.phase = Status::Idle;
                session.retired.clear();
                let why = match event.payload["status"].as_str() {
                    Some("interrupted") => Closed::Interrupted,
                    _ => Closed::TurnEnded,
                };
                // Only what was asked in the turn that ended, or before it:
                // a completion read late must not take a newer turn's card.
                // A set of questions names no turn; Hermes withdraws it.
                let Some(ended) = event.generation else {
                    return;
                };
                for id in self.asked(&at, |ask| ask.generation().is_some_and(|g| g <= ended)) {
                    self.close(&id, why.clone(), effects);
                }
            }
            _ => {}
        }
    }

    /// Shows a request, unless it is shown already or was closed this turn.
    pub(super) fn show(
        &mut self,
        at: SessionRef,
        ask: Ask,
        deadline_ms: Option<i64>,
        effects: &mut Vec<Effect>,
    ) {
        let id = at.request_id(ask.key());
        let retired = self
            .sessions
            .get(&(at.profile.clone(), at.id.clone()))
            .is_some_and(|session| session.retired.contains(&id));
        if retired || self.open.contains_key(&id) {
            return;
        }
        let pending = Pending {
            session: at,
            ask,
            deadline_ms,
        };
        effects.push(Effect::Open(pending.clone()));
        self.open.insert(
            id,
            Open {
                pending,
                sent_ms: None,
            },
        );
    }

    pub(super) fn close(&mut self, id: &str, why: Closed, effects: &mut Vec<Effect>) {
        let Some(open) = self.open.remove(id) else {
            return;
        };
        let at = open.pending.session;
        if let Some(session) = self.sessions.get_mut(&(at.profile.clone(), at.id.clone())) {
            session.retired.insert(id.to_owned());
        }
        effects.push(Effect::Close {
            id: id.to_owned(),
            session: at.hud_id(),
            why,
        });
    }

    /// Forgets everything about the gateway's sessions, closing every card.
    pub(super) fn reset(&mut self, why: Closed, effects: &mut Vec<Effect>) {
        for id in self.open.keys().cloned().collect::<Vec<_>>() {
            self.close(&id, why.clone(), effects);
        }
        self.sessions.clear();
    }

    /// The ids of the session's open requests that `which` picks.
    pub(super) fn asked(&self, at: &SessionRef, which: impl Fn(&Ask) -> bool) -> Vec<String> {
        self.open
            .iter()
            .filter(|(_, open)| open.pending.session == *at && which(&open.pending.ask))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub(super) fn entry_mut(&mut self, profile: &Profile, id: &str) -> &mut Session {
        self.sessions
            .entry((profile.clone(), id.to_owned()))
            .or_insert_with(|| Session {
                phase: Status::Idle,
                cursor: None,
                subscription: None,
                snapshot_at: None,
                retired: BTreeSet::new(),
            })
    }
}
