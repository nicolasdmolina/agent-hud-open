//! What Approve automatically (`auto.rs`) never does (Agent HUD Fork,
//! ticket 66):
//!
//! - it never answers a request of a client whose requests always wait
//!   for the person (`Channel::answered_by_hand`, as its agent says);
//! - it never decides a request whose card says its client cannot be
//!   reached (`unreachable`): that request takes no decision at all
//!   (`answer.rs`), an automatic one included;
//! - a request it allowed showed no card, so it makes no sound. The
//!   session may still be told as waiting for a moment, by its hooks or
//!   its channel, until its client goes on; core keeps which sessions were
//!   just approved in, and the sound decision (`sound_alerts`) lets such
//!   a wait pass while it has no card.
use super::{Requests, State};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::PoisonError;

/// How long after an automatic approval a wait with no card in its session
/// is taken as that request's: its client has the answer within moments,
/// and its channel's next reading says so.
pub const APPROVED_QUIET_MS: i64 = 5_000;

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
    /// its session is noted.
    pub(super) fn approved_at_once(&mut self, id: &str, view: &Value, now_ms: i64) -> bool {
        let source = view["source"].as_str().unwrap_or_default();
        let approved = view["kind"].as_str().unwrap_or("permission") == "permission"
            && takes_a_decision(view)
            && self.auto.approves(source, view["client"].as_str());
        if approved {
            self.dismissed.insert(id.to_owned());
            let session = view["sessionId"].as_str().unwrap_or_default();
            self.note_approved(source, session, now_ms);
        }
        approved
    }

    pub(super) fn note_approved(&mut self, source: &str, session: &str, now_ms: i64) {
        self.approved
            .retain(|_, at| now_ms.saturating_sub(*at) < APPROVED_QUIET_MS);
        if !source.is_empty() && !session.is_empty() {
            self.approved.insert(format!("{source}:{session}"), now_ms);
        }
    }
}

impl Requests {
    /// Notes that a permission request of `session` was allowed at once,
    /// with no card: what a hook's callback tells, where the request never
    /// came to `register`.
    pub fn note_auto_approved(&self, source: &str, session: &str, now_ms: i64) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note_approved(source, session, now_ms);
    }

    /// The sessions a permission was allowed in with no card within
    /// `APPROVED_QUIET_MS` before `now_ms`, by `source:session`, each with
    /// when.
    pub fn auto_approved(&self, now_ms: i64) -> HashMap<String, i64> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .approved
            .iter()
            .filter(|(_, at)| now_ms.saturating_sub(**at) < APPROVED_QUIET_MS)
            .map(|(key, at)| (key.clone(), *at))
            .collect()
    }
}

#[cfg(test)]
#[path = "auto_limits_tests.rs"]
mod tests;
