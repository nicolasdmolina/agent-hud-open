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
//! - a request it allowed showed no card, so it plays no sound. While the
//!   client takes the answer, its hooks or its channel can still tell the
//!   session as waiting, and a wait with no card would sound. So core
//!   keeps each approved request, by its session and under its own names
//!   (its id, and its tool call where it names one), until the sound
//!   decision (`sound_alerts`) has next looked at that session. That look
//!   decides the request's names, so nothing that goes by them sounds, and
//!   takes a wait with no card that it finds there as that request's. No
//!   clock is in it: a wait seen late is still the request's, and once a
//!   look has seen the session, the next wait is a new one.
use super::{Requests, State};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::PoisonError;

/// How many approved requests are kept for looks that have not come: only
/// so that a session that is never listed cannot make the list grow.
const KEPT: usize = 512;

/// A request that was allowed at once, with no card, until the sound
/// decision has looked at its session.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Approved {
    /// `source:session`.
    session: String,
    /// The names the request goes by: its id, and `call:<id>` for the
    /// tool call it names, as the sound decision names a card's.
    names: Vec<String>,
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
    pub(super) fn approved_at_once(&mut self, id: &str, view: &Value) -> bool {
        let source = view["source"].as_str().unwrap_or_default();
        let approved = view["kind"].as_str().unwrap_or("permission") == "permission"
            && takes_a_decision(view)
            && self.auto.approves(source, view["client"].as_str());
        if approved {
            self.dismissed.insert(id.to_owned());
            let session = view["sessionId"].as_str().unwrap_or_default();
            self.note_approved(source, session, Some(id), view["callId"].as_str());
        }
        approved
    }

    pub(super) fn note_approved(
        &mut self,
        source: &str,
        session: &str,
        id: Option<&str>,
        call: Option<&str>,
    ) {
        if source.is_empty() || session.is_empty() {
            return;
        }
        let names = [id.map(str::to_owned), call.map(|c| format!("call:{c}"))];
        self.approved.push_back(Approved {
            session: format!("{source}:{session}"),
            names: names.into_iter().flatten().collect(),
        });
        while self.approved.len() > KEPT {
            self.approved.pop_front();
        }
    }
}

impl Requests {
    /// Notes that a permission request of `session` was allowed at once,
    /// with no card: what a hook's callback tells, where the request never
    /// came to `register`. `call` is the tool call it names, where it does.
    pub fn note_auto_approved(&self, source: &str, session: &str, call: Option<&str>) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note_approved(source, session, None, call);
    }

    /// The requests allowed with no card that the sound decision has not
    /// looked at the session of yet: by `source:session`, the names each
    /// goes by. A session is there for each of them, though it name none.
    pub fn auto_approved(&self) -> HashMap<String, Vec<String>> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut kept: HashMap<String, Vec<String>> = HashMap::new();
        for approved in &state.approved {
            let names = kept.entry(approved.session.clone()).or_default();
            names.extend(approved.names.iter().cloned());
        }
        kept
    }

    /// The sound decision has looked at these sessions (`source:session`):
    /// what was kept of them is over.
    pub fn auto_approved_seen<'a>(&self, sessions: impl IntoIterator<Item = &'a str>) {
        let seen: std::collections::HashSet<&str> = sessions.into_iter().collect();
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.approved.is_empty() {
            state
                .approved
                .retain(|approved| !seen.contains(approved.session.as_str()));
        }
    }
}

#[cfg(test)]
#[path = "auto_limits_tests.rs"]
mod tests;
