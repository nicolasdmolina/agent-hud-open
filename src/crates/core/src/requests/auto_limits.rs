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
//!   request plays no sound. Core keeps each approved request under its
//!   own names (its id, and its tool call where it names one). The sound
//!   decision (`sound_alerts`) is silent only for what goes by one of
//!   those names: a wait its channel says is for that request
//!   (`set_native_waits`), or a transcript sighting of that call. Nothing
//!   is inferred from the order of things or from a clock: a wait that
//!   names no request always sounds, though it may be an approved
//!   request's. An extra sound is the lesser fault; a silent wait is not.
use super::{Requests, State};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::PoisonError;

/// How long an approved request is kept where the sound decision never
/// sees its wait end: ten minutes, which is the longest a request waits
/// for an answer by default. Only so that the list cannot grow: a name
/// belongs to one request, so one kept too long silences nothing else.
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
    /// it is kept for the sound decision.
    pub(super) fn approved_at_once(&mut self, id: &str, view: &Value, now_ms: i64) -> bool {
        let source = view["source"].as_str().unwrap_or_default();
        let approved = view["kind"].as_str().unwrap_or("permission") == "permission"
            && takes_a_decision(view)
            && self.auto.approves(source, view["client"].as_str());
        if approved {
            self.dismissed.insert(id.to_owned());
            let session = view["sessionId"].as_str().unwrap_or_default();
            let call = view["callId"].as_str();
            self.note_approved(source, session, Some(id), call, now_ms);
        }
        approved
    }

    pub(super) fn note_approved(
        &mut self,
        source: &str,
        session: &str,
        id: Option<&str>,
        call: Option<&str>,
        now_ms: i64,
    ) {
        let names = [id.map(str::to_owned), call.map(|c| format!("call:{c}"))];
        let names: Vec<String> = names.into_iter().flatten().collect();
        if source.is_empty() || session.is_empty() || names.is_empty() {
            return;
        }
        self.approved.push_back(Approved {
            session: format!("{source}:{session}"),
            names,
            at_ms: now_ms,
        });
        while self.approved.len() > APPROVED_KEPT {
            self.approved.pop_front();
        }
    }
}

impl Requests {
    /// Notes that a permission request of `session` was allowed at once,
    /// with no card: what a hook's callback tells, where the request never
    /// came to `register`. `call` is the tool call it names; one that names
    /// none has nothing to be known by, and is not kept.
    pub fn note_auto_approved(&self, source: &str, session: &str, call: Option<&str>, now_ms: i64) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note_approved(source, session, None, call, now_ms);
    }

    /// The requests allowed with no card that are still kept at `now_ms`:
    /// by `source:session`, the names each goes by.
    pub fn auto_approved(&self, now_ms: i64) -> HashMap<String, Vec<String>> {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .approved
            .retain(|approved| now_ms.saturating_sub(approved.at_ms) < APPROVED_KEPT_MS);
        let mut kept: HashMap<String, Vec<String>> = HashMap::new();
        for approved in &state.approved {
            let names = kept.entry(approved.session.clone()).or_default();
            names.extend(approved.names.iter().cloned());
        }
        kept
    }

    /// The sound decision saw the waits that went by these names end: the
    /// requests they belong to are over. Only those: a request approved
    /// since the names were read is kept, whatever its session.
    pub fn auto_approved_ended(&self, names: &[String]) {
        if names.is_empty() {
            return;
        }
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .approved
            .retain(|approved| !approved.names.iter().any(|name| names.contains(name)));
    }
}

#[cfg(test)]
#[path = "auto_limits_tests.rs"]
mod tests;
