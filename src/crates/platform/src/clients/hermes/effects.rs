//! Doing what the model says: each effect becomes a call on core's
//! requests. A card opens once under its request id, changes in place, and
//! is withdrawn when Hermes says the request is over. It is never
//! withdrawn for being out of reach: it is marked in place, and a card
//! core put away for an answer that did not land is given back as it was
//! (`back`), never registered anew. What the sessions are doing is told to
//! core in one place, `tell`, after everything the model takes in, under
//! the follower's own reader name.
use super::{answer::NO_REPLY, Model, Shared};
use agent_hud_agents::hermes::{
    gateway::{Ask, Closed, Connection, Effect, Gateway, Input, Pending, Refusal},
    AGENT,
};
use agent_hud_core::l10n;
use serde_json::Value;

/// The machine label on the cards made here: none, since every gateway
/// this channel follows runs on this machine.
const MACHINE: Option<&str> = None;

impl Shared {
    /// Carries out `effects` in order. Called with the model locked, so two
    /// threads never interleave what they were told to do.
    pub(super) fn carry(&self, model: &mut Model, effects: Vec<Effect>, now: i64) {
        if effects.is_empty() {
            return;
        }
        for effect in effects {
            match effect {
                Effect::Connection(connection) => self.log.line(&said(&connection)),
                Effect::Open(pending) => {
                    self.log.line(&format!(
                        "open id={} kind={} session={}",
                        pending.id(),
                        kind(&pending),
                        pending.session.hud_id()
                    ));
                    self.show(model, &pending, now);
                }
                Effect::Changed(pending) => {
                    let id = pending.id();
                    self.log.line(&format!("change id={id}"));
                    let view = self.view(model, &pending, now);
                    self.requests.refresh_native(&id, view);
                }
                // Out of reach, or found waiting again: the same request,
                // changed in place. Its time, its hold and whether it
                // was dismissed are core's, and are not touched.
                Effect::Marked(pending) | Effect::Unmarked(pending) => {
                    let id = pending.id();
                    let marked = model.gateway.marked(&id);
                    let word = if marked { "unreachable" } else { "reached" };
                    self.log.line(&format!("{word} id={id}"));
                    let view = self.view(model, &pending, now);
                    self.requests.refresh_native(&id, view);
                }
                Effect::Close { id, session, why } => {
                    self.log.line(&format!("close id={id} why={why:?}"));
                    model.waiters.remove(&id);
                    model.opened.remove(&id);
                    model.taken.remove(&id);
                    // An answer to it that is being written is cut.
                    model.cut_writing(|request| request == Some(id.as_str()));
                    self.requests.settle_native(&id);
                    self.closed(model, &id, &session.hud_id(), &why, now);
                }
                Effect::Unanswered { pending, why } => {
                    let id = pending.id();
                    self.log.line(&format!("unanswered id={id} why={why:?}"));
                    model.answered.remove(&id);
                    // Core put the card away when it was answered; this
                    // gives it back as it was, since the request still waits.
                    self.back(model, &pending, now);
                    let (state, reason) = match &why {
                        Some(Refusal::Other(word)) if word == NO_REPLY => {
                            ("uncertain", unconfirmed())
                        }
                        Some(refusal) => ("failed", refused(refusal)),
                        None => ("uncertain", unconfirmed()),
                    };
                    self.note(&id, &pending.session.hud_id(), state, Some(reason), now);
                }
                Effect::Poll => self.want_poll(),
            }
        }
        (self.wake)();
    }

    /// An answer that could not be sent at all (the decision taken in
    /// `wait`): nothing of it was written. Its card comes back as it was,
    /// marked, for as long as Hermes still waits on it. Nothing comes back
    /// where the request is over meanwhile, the follower ended, or the
    /// answer is not the one that is out now.
    pub(super) fn undelivered(&self, pending: &Pending, wait: u64, reason: String) {
        let mut model = self.model();
        let id = pending.id();
        let out = model.taken.get(&id) == Some(&wait);
        if model.stopped || model.gateway.pending(&id).is_none() || !out {
            return;
        }
        let now = crate::hooks::now_ms();
        self.log.line(&format!("undelivered id={id}"));
        // The gateway took no answer just now: the model marks the card
        // until a poll finds the request waiting again.
        self.take(&mut model, Input::Unsent { request: &id });
        self.back(&mut model, pending, now);
        self.note(&id, &pending.session.hud_id(), "failed", Some(reason), now);
        (self.wake)();
    }

    /// The card as it shows now: marked while its gateway is out of reach.
    fn view(&self, model: &Model, pending: &Pending, now: i64) -> Value {
        let id = pending.id();
        let opened = model.opened.get(&id).copied().unwrap_or(now);
        if model.gateway.marked(&id) {
            pending.view_out_of_reach(opened, MACHINE)
        } else {
            pending.view(opened, MACHINE)
        }
    }

    /// Registers the card with core and keeps where its answer will arrive.
    /// Core refuses one it already holds, which changes nothing.
    fn show(&self, model: &mut Model, pending: &Pending, now: i64) {
        let id = pending.id();
        model.opened.entry(id.clone()).or_insert(now);
        let view = self.view(model, pending, now);
        #[cfg(test)]
        (self.shows).fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let Some(waiter) = self.requests.register_native(id.clone(), view, now) {
            model.waiters.insert(id, waiter);
        }
    }

    /// Gives core back a card it put away for an answer that did not land:
    /// the same request, with the time it came and the hold that runs from
    /// it, and not dismissed (`Requests::put_back`). It is not registered
    /// again, which would start its hold anew. Nothing comes back where
    /// the request came to its end for core meanwhile (its hold ran out),
    /// or where the answer that is out is no longer this one.
    fn back(&self, model: &mut Model, pending: &Pending, now: i64) {
        let id = pending.id();
        let Some(wait) = model.taken.remove(&id) else {
            return;
        };
        let view = self.view(model, pending, now);
        match self.requests.put_back(&id, wait, view, now) {
            Some(waiter) => {
                model.waiters.insert(id, waiter);
            }
            None => self.log.line(&format!("over id={id}")),
        }
    }

    /// What became of an answer given here, once its card closed.
    pub(super) fn receipt(&self, id: &str, session: &str, why: &Closed, batch: bool, now: i64) {
        match why {
            Closed::Answered => self.note(id, session, "confirmed", None, now),
            // Someone answered first, or the turn was over.
            Closed::AlreadyResolved | Closed::StaleGeneration => {
                self.note(id, session, "failed", Some(too_late()), now)
            }
            // Nothing replies to the answer to a set of questions: the set
            // being over after it was written is all the receipt there is.
            Closed::Gone | Closed::Settled | Closed::Withdrawn(_) if batch => {
                self.note(id, session, "confirmed", None, now)
            }
            _ => {}
        }
    }

    pub(super) fn note(
        &self,
        id: &str,
        session: &str,
        state: &str,
        reason: Option<String>,
        now: i64,
    ) {
        self.requests
            .note_native_delivery(id, AGENT.id, session, state, reason, now);
    }

    /// Tells core what the gateway's sessions are doing: which of them wait
    /// on a card, and what each is doing as the newest poll that counted
    /// listed it (`live_states`, ticket 16). It is called after everything
    /// the model takes in, changed or not, since core ignores what it
    /// already knows. Each follower tells under its own name, its
    /// gateway's id, the same for its whole life: core keeps the readings
    /// of two followers apart, each with its own time, and nothing here
    /// joins them (ticket 24). A follower that ends says so once
    /// (`follower.rs`, `end`), which clears what it told and nothing else.
    pub(super) fn tell(&self, gateway: &Gateway, now: i64) {
        let mut waiting: Vec<String> = gateway
            .waiting()
            .map(|pending| pending.session.hud_id())
            .collect();
        waiting.sort();
        waiting.dedup();
        let requests = &self.requests;
        requests.set_native_waiting(AGENT.id, &self.reader, &waiting, now);
        requests.set_native_states(AGENT.id, &self.reader, &gateway.live_states());
    }
}

fn kind(pending: &Pending) -> &'static str {
    match pending.ask {
        Ask::Question(_) => "question",
        Ask::Approval(_) => "approval",
        Ask::Batch(_) => "questions",
    }
}

/// The connection's state as one log line. An instance is named by the
/// start of its id, which is no credential.
fn said(connection: &Connection) -> String {
    match connection {
        Connection::Disconnected => "connection disconnected".into(),
        Connection::Connecting => "connection connecting".into(),
        Connection::Ready(instance) => format!(
            "connection ready instance={} profiles={} draining={}",
            instance.id.chars().take(8).collect::<String>(),
            instance.served.len(),
            instance.draining
        ),
        Connection::Unsupported(why) => {
            format!("connection unsupported reason={}", why.reason_code())
        }
        Connection::Failed(failure) => format!("connection failed reason={failure:?}"),
    }
}

fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::InvalidParams => l10n::text(
            "Hermes 不接受这个回答",
            "Hermes did not accept that answer.",
        ),
        Refusal::PermissionDenied => l10n::text(
            "Hermes 不允许 Agent HUD 回答这个会话",
            "Hermes did not let Agent HUD answer this session.",
        ),
        _ => l10n::text(
            "Hermes 暂时无法接收这个回答",
            "Hermes could not take the answer just now.",
        ),
    }
}

fn unconfirmed() -> String {
    l10n::text(
        "Hermes 没有确认收到这个回答",
        "Hermes did not confirm receipt of the answer.",
    )
}

pub(super) fn too_late() -> String {
    l10n::text(
        "Hermes 已不再等待这个回答",
        "Hermes was no longer waiting for this answer.",
    )
}
