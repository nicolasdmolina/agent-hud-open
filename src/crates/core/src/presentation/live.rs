//! What a client's native channel says of its sessions, laid over what
//! their hooks and logs say. A card the channel holds makes its session
//! wait. Otherwise the state the channel read stands, for the sessions it
//! names, while two things hold: the reading is recent
//! (`requests::LIVE_STATES_MS`), and the session's hooks have reported
//! nothing since it was taken. A hook's running turn lapses to idle after
//! two minutes of quiet, and a channel that reads the client live knows
//! better; but a turn end or start the hooks push after the reading is
//! newer news than the reading, until the channel reads again. A session
//! the channel leaves out, a channel that said nothing, went away or
//! stopped reading keeps what the hooks and the log say.
use crate::hook_events::HookEvidence;
use crate::requests::{NativeStates, Requests};
use agent_hud_agents::LiveState;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(super) struct Native<'a> {
    /// The sessions a native card waits in, by source.
    waiting: HashMap<String, HashSet<String>>,
    /// What each source's channel read its live sessions doing, and when.
    states: HashMap<String, NativeStates>,
    hooks: &'a HookEvidence,
    now: i64,
}

impl<'a> Native<'a> {
    pub(super) fn of(requests: &Requests, hooks: &'a HookEvidence, now: i64) -> Self {
        Self {
            waiting: requests.native_waiting_sessions(),
            states: requests.native_states(),
            hooks,
            now,
        }
    }
    fn waiting(&self, source: &str, id: &str) -> bool {
        self.waiting
            .get(source)
            .is_some_and(|sessions| sessions.contains(id))
    }
    fn told(&self, source: &str, id: &str) -> Option<LiveState> {
        let states = self.states.get(source).filter(|s| s.fresh(self.now))?;
        let state = *states.sessions.get(id)?;
        let reported = self.hooks.reported_at(source, id, self.now);
        let newer = reported.is_some_and(|at| at > states.read_ms);
        (!newer).then_some(state)
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
