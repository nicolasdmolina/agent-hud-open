//! What a client's native channel says of its sessions, laid over what
//! their hooks and logs say. A card the channel holds makes its session
//! wait. Otherwise the state the channel reports stands, for the sessions
//! it names and only while it names them: a hook's running turn lapses to
//! idle after two minutes of quiet, and a channel that reads the client
//! live knows better. A session the channel leaves out, or a channel that
//! said nothing or went away, keeps what the hooks and the log say.
use crate::requests::{NativeStates, Requests};
use agent_hud_agents::LiveState;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(super) struct Native {
    /// The sessions a native card waits in, by source.
    waiting: HashMap<String, HashSet<String>>,
    /// What each source's channel says its live sessions are doing.
    states: HashMap<String, NativeStates>,
}

impl Native {
    pub(super) fn of(requests: &Requests) -> Self {
        Self {
            waiting: requests.native_waiting_sessions(),
            states: requests.native_states(),
        }
    }
    fn waiting(&self, source: &str, id: &str) -> bool {
        self.waiting
            .get(source)
            .is_some_and(|sessions| sessions.contains(id))
    }
    fn told(&self, source: &str, id: &str) -> Option<LiveState> {
        self.states.get(source)?.get(id).copied()
    }
    /// The state the channel gives a session, where it gives one.
    pub(super) fn state(&self, source: &str, id: &str) -> Option<LiveState> {
        if self.waiting(source, id) {
            return Some(LiveState::Waiting);
        }
        self.told(source, id)
    }
    /// Whether the channel says the session is in a turn.
    pub(super) fn busy(&self, source: &str, id: &str) -> bool {
        matches!(
            self.state(source, id),
            Some(LiveState::Running | LiveState::Waiting)
        )
    }
    /// Lays the channel's state over a session row the hooks and the log
    /// made. A turn the channel says is under way has not ended, whatever
    /// an earlier turn's end left in the facts.
    pub(super) fn overlay(&self, session: &mut Value) {
        let source = session["source"].as_str().unwrap_or("").to_owned();
        let id = session["id"].as_str().unwrap_or("").to_owned();
        if self.waiting(&source, &id) {
            session["state"] = json!("waiting");
            return;
        }
        let Some(state) = self.told(&source, &id) else {
            return;
        };
        session["state"] = json!(state.word());
        let facts = &mut session["liveFacts"];
        if state != LiveState::Idle && facts.is_object() {
            facts["endedAt"] = Value::Null;
            facts["outcome"] = Value::Null;
        }
    }
}
