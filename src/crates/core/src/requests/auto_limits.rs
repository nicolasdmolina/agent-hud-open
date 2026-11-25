//! What Approve automatically (`auto.rs`) never does (Agent HUD Fork,
//! ticket 66):
//!
//! - it never answers a request of a client whose requests always wait
//!   for the person (`Channel::answered_by_hand`, as its agent says);
//! - it never decides a request whose card says its client cannot be
//!   reached (`unreachable`): that request takes no decision at all
//!   (`answer.rs`), an automatic one included;
//! - it never allows a call the person's own rule in the client says to
//!   ask about (`agents::hooks::RULE_ASKS`). That one is held where the
//!   mark is read, in the hook's callback
//!   (`platform/src/hooks/callback.rs`), since only a hook carries it;
//! - a request it allowed showed no card, so what is known to be that
//!   request plays no sound, once its answer is known to have arrived.
//!   Core keeps each approved request under its own names (its id, and
//!   its tool call where it names one), and says it is approved only
//!   after its channel confirmed that the automatic answer was delivered
//!   (`auto_answer_delivered`, by the number of the wait the answer was
//!   given in, or a hook's reply written to the hook that asked). Until then, and where the answer failed, was not confirmed,
//!   was never sent because its listener stopped, or was not heard of
//!   in ten minutes, the request is not approved: its wait and its card
//!   sound as they did before the sync. The sound decision
//!   (`sound_alerts`) asks at every look and remembers no approval. It
//!   is silent only for what goes by a confirmed request's names: a
//!   wait its channel says is for that request (`set_native_waits`), or
//!   a transcript sighting of that call. Nothing is inferred from the
//!   order of things: a wait that names no request always sounds. An
//!   extra sound is the lesser fault; a silent wait is not.
use super::{Requests, State};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::PoisonError;

/// How long an approved request is kept: ten minutes, which is the
/// longest a request waits for an answer by default. One whose answer is
/// not confirmed by then never counts as approved. A confirmed one is
/// kept past it only while its channel still says a wait is for it, so
/// that the same wait does not start to sound. A name belongs to one
/// request, so one kept too long silences nothing else.
pub const APPROVED_KEPT_MS: i64 = 600_000;
/// How many are kept at most, the oldest going first: for the same reason.
pub const APPROVED_KEPT: usize = 512;

/// A request that was allowed at once, with no card.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Approved {
    /// `source:session`.
    session: String,
    /// The names the request goes by: its id, and `call:<id>` for the
    /// tool call it names, as the sound decision names a card's.
    names: Vec<String>,
    at_ms: i64,
    /// The id it was registered under, where a native channel did
    /// (`register_native`): what its channel settles it by.
    native: Option<String>,
    /// The wait its automatic answer was given in (its ticket's number),
    /// where a native channel registered it. Only word of that very
    /// answer confirms it: an id may be used again, a wait's number never.
    wait: Option<u64>,
    /// Its channel confirmed that the automatic answer was delivered. Only
    /// then is the request approved for the sound decision.
    confirmed: bool,
}

/// Whether the requests of `agent` always wait for the person.
pub(super) fn by_hand(agent: &str) -> bool {
    agent_hud_agents::answered_by_hand(agent)
}

/// Whether the request with this card takes a decision now: not while it
/// says its client cannot be reached.
pub(super) fn takes_a_decision(view: &Value) -> bool {
    matches!(view.get("unreachable"), None | Some(Value::Null))
}

impl State {
    /// Whether the native request `id` is allowed at once, as upstream's
    /// rule has it (a permission whose client approves automatically) and
    /// within the limits above. Where it is, it is never shown again, and
    /// it is kept, not yet confirmed, for the sound decision, under the
    /// number of the wait its answer goes out in (the ticket's).
    pub(super) fn approved_at_once(&mut self, id: &str, view: &Value, now_ms: i64) -> Option<u64> {
        let source = view["source"].as_str().unwrap_or_default();
        let approved = view["kind"].as_str().unwrap_or("permission") == "permission"
            && takes_a_decision(view)
            && self.auto.approves(source, view["client"].as_str());
        if !approved {
            return None;
        }
        self.dismissed.insert(id.to_owned());
        let session = view["sessionId"].as_str().unwrap_or_default();
        let call = view["callId"].as_str();
        let wait = self.begin_wait();
        self.note_approved(source, session, Some((id, wait)), call, now_ms);
        Some(wait)
    }

    pub(super) fn note_approved(
        &mut self,
        source: &str,
        session: &str,
        native: Option<(&str, u64)>,
        call: Option<&str>,
        now_ms: i64,
    ) {
        let id = native.map(|(id, _)| id);
        let names = [id.map(str::to_owned), call.map(|c| format!("call:{c}"))];
        let names: Vec<String> = names.into_iter().flatten().collect();
        if source.is_empty() || session.is_empty() || names.is_empty() {
            return;
        }
        self.approved.push_back(Approved {
            session: format!("{source}:{session}"),
            names,
            at_ms: now_ms,
            native: id.map(str::to_owned),
            wait: native.map(|(_, wait)| wait),
            // A hook's approval is noted once its reply was delivered; a
            // native one waits for word of its answer.
            confirmed: native.is_none(),
        });
        while self.approved.len() > APPROVED_KEPT {
            self.approved.pop_front();
        }
    }
}

impl State {
    /// Its channel settled the native request `id`: it is over there, or
    /// its listener stopped before the answer left. Either way nothing is
    /// kept of it, so the same request shown again is a request like any
    /// other, and sounds.
    pub(super) fn approval_settled(&mut self, id: &str) {
        self.approved
            .retain(|approved| approved.native.as_deref() != Some(id));
    }
}

impl Requests {
    /// Notes that a permission request of `session` was allowed at once,
    /// with no card, and that the answer reached the hook that asked: what
    /// a hook's callback tells once its reply is written, where the
    /// request never came to `register`. `call` is the tool call it names;
    /// one that names none has nothing to be known by, and is not kept.
    pub fn note_auto_approved(&self, source: &str, session: &str, call: Option<&str>, now_ms: i64) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note_approved(source, session, None, call, now_ms);
    }

    /// The channel saw the automatic answer that was given in the wait
    /// numbered `wait` (its ticket's) delivered: the request it was given
    /// to is approved from now on. Word of any other answer says nothing
    /// of it, though that answer was for a request of the same id in the
    /// same session: one settled before, whose answer was still on its
    /// way. One not heard of in ten minutes is approved no more, and no
    /// word that comes after makes it so: it goes here, though no look
    /// came in between to drop it. A wait that was answered by hand, or
    /// whose request is settled, has nothing kept under its number.
    pub fn auto_answer_delivered(&self, wait: u64, now_ms: i64) {
        let confirmed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let in_time = |a: &Approved| now_ms.saturating_sub(a.at_ms) < APPROVED_KEPT_MS;
            let mine = |a: &Approved| a.wait == Some(wait);
            state
                .approved
                .retain(|a| !mine(a) || a.confirmed || in_time(a));
            let mut confirmed = false;
            for approved in state.approved.iter_mut().filter(|a| mine(a)) {
                confirmed |= !std::mem::replace(&mut approved.confirmed, true);
            }
            if confirmed {
                state.revision = state.revision.wrapping_add(1);
            }
            confirmed
        };
        if confirmed {
            self.wake();
        }
    }

    /// The requests allowed with no card whose answer is confirmed
    /// delivered, as core holds them at `now_ms`: by `source:session`, the
    /// names each goes by. This is the whole of what counts as approved
    /// for sound, asked anew at every look.
    pub fn auto_approved(&self, now_ms: i64) -> HashMap<String, Vec<String>> {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let waits = state.native.wait_requests();
        let waited_for = |approved: &Approved| {
            let named = waits.get(&approved.session);
            named.is_some_and(|ids| approved.names.iter().any(|name| ids.contains(name)))
        };
        state.approved.retain(|approved| {
            now_ms.saturating_sub(approved.at_ms) < APPROVED_KEPT_MS
                || (approved.confirmed && waited_for(approved))
        });
        let approved = &state.approved;
        let mut kept: HashMap<String, Vec<String>> = HashMap::new();
        for approved in approved.iter().filter(|approved| approved.confirmed) {
            let names = kept.entry(approved.session.clone()).or_default();
            names.extend(approved.names.iter().cloned());
        }
        kept
    }

    /// The sound decision saw the waits of `session` (`source:session`)
    /// that went by these names end: the requests they belong to are over.
    /// Only those, and only there: a request approved since the names were
    /// read is kept, and so is one of another source or session that goes
    /// by the same name.
    pub fn auto_approved_ended(&self, session: &str, names: &[String]) {
        if names.is_empty() {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.approved.retain(|approved| {
            approved.session != session || !approved.names.iter().any(|name| names.contains(name))
        });
    }
}

#[cfg(test)]
#[path = "auto_limits_tests.rs"]
mod tests;
