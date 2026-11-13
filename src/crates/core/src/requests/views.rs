//! What waits on the user, as the HUD and the hosts read it: the cards, the
//! sessions they belong to, and when they next change.
use super::{Content, Requests};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::PoisonError,
};

/// How long a native channel's reading of its sessions is believed after it
/// came back. The channel that tells states today reads its client every
/// second while anything runs and every ten seconds when all is idle;
/// thirty seconds is three idle readings, room for a slow one. A channel that stops reading without a word
/// (a call that hangs on a socket that still looks open) gives its sessions
/// back to their hooks and logs by then.
pub const LIVE_STATES_MS: i64 = 30_000;

/// What a native channel last said of its live sessions, and when it read
/// them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeStates {
    pub read_ms: i64,
    pub sessions: HashMap<String, agent_hud_agents::LiveState>,
}

impl NativeStates {
    /// Whether the reading is still believed at `now_ms`.
    pub fn fresh(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.read_ms) < LIVE_STATES_MS
    }
}

impl Requests {
    /// The first time a waiting request expires or a delivery leaves the
    /// hour it is kept for; `views` and `deliveries` change then.
    pub fn next_deadline(&self) -> Option<i64> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let deliveries = state.deliveries.iter().filter_map(|row| {
            row["at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|at| at.timestamp_millis().saturating_add(3_600_000))
        });
        state
            .pending
            .iter()
            .map(|item| item.deadline_ms)
            .chain(deliveries)
            .chain(state.native_states.values().map(|states| {
                // When the reading lapses.
                states.read_ms.saturating_add(LIVE_STATES_MS)
            }))
            .min()
    }
    pub fn revision(&self) -> u64 {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .revision
    }
    pub fn deadline(&self, id: &str) -> Option<i64> {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.deadline_ms)
    }
    pub fn waiting_at(&self, source: &str, session: &str) -> Option<i64> {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .iter()
            .filter(|item| match &item.content {
                Content::Hook(request) => request.source == source && request.session_id == session,
                Content::Native(view) => view["source"] == source && view["sessionId"] == session,
            })
            .map(|item| item.created_ms)
            .min()
    }
    pub fn attention(&self, source: &str, session: &str) -> Vec<Value> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner).pending.iter().filter_map(|item| {
            let kind = match &item.content {
                Content::Hook(request) if request.source == source && request.session_id == session => request.kind.as_str(),
                Content::Native(view) if view["source"] == source && view["sessionId"] == session => view["kind"].as_str().unwrap_or("permission"),
                _ => return None,
            };
            Some(serde_json::json!({"id":item.id,"sessionId":session,"kind":kind,"at":chrono::DateTime::<chrono::Utc>::from_timestamp_millis(item.created_ms).map(|v|v.to_rfc3339())}))
        }).collect()
    }
    /// Native waiting flags may exist without a HUD-answerable request. Keep
    /// phase evidence here without fabricating a card or a transcript timestamp.
    pub fn set_native_waiting(&self, source: &str, sessions: &[String], _now_ms: i64) {
        let next: HashSet<String> = sessions
            .iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect();
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state
                .native_waiting
                .get(source)
                .is_none_or(|old| *old != next)
                && (!next.is_empty() || state.native_waiting.contains_key(source))
            {
                if next.is_empty() {
                    state.native_waiting.remove(source);
                } else {
                    state.native_waiting.insert(source.into(), next);
                }
                state.revision = state.revision.wrapping_add(1);
                true
            } else {
                false
            }
        };
        if changed {
            self.wake();
        }
    }
    /// What `source`'s native channel says each live session is doing,
    /// whole, as of `states.read_ms`: a session it leaves out, or an empty
    /// list, goes back to what its hooks and its log say, and so does every
    /// session once the reading is `LIVE_STATES_MS` old. A channel may call
    /// this after everything it does: the very same reading told again
    /// changes nothing. Any newer reading wakes the HUD, even of the same
    /// sessions, since it decides again against hooks that reported since
    /// the last one (a channel reads at most about once a second).
    pub fn set_native_states(&self, source: &str, states: &agent_hud_agents::LiveStates) {
        let sessions: HashMap<_, _> = states
            .sessions
            .iter()
            .filter(|(id, _)| !id.is_empty())
            .cloned()
            .collect();
        let read_ms = states.read_ms;
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let renewed = match state.native_states.get_mut(source) {
                None if sessions.is_empty() => false,
                // The same sessions, read again and not lapsed meanwhile.
                Some(old) if old.sessions == sessions && old.fresh(read_ms) => {
                    let newer = read_ms > old.read_ms;
                    old.read_ms = old.read_ms.max(read_ms);
                    newer
                }
                _ => {
                    if sessions.is_empty() {
                        state.native_states.remove(source);
                    } else {
                        let next = NativeStates { read_ms, sessions };
                        state.native_states.insert(source.into(), next);
                    }
                    true
                }
            };
            if renewed {
                state.revision = state.revision.wrapping_add(1);
            }
            renewed
        };
        if changed {
            self.wake();
        }
    }
    pub fn native_states(&self) -> HashMap<String, NativeStates> {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .native_states
            .clone()
    }
    pub fn native_waiting_sessions(&self) -> HashMap<String, HashSet<String>> {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .native_waiting
            .clone()
    }
    /// The tool call each waiting card holds, by card id, where its client
    /// names it (`call_id`, a native card's `callId`).
    pub fn calls(&self) -> HashMap<String, String> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let call = |item: &super::Pending| match &item.content {
            Content::Hook(request) => request.call_id.clone(),
            Content::Native(view) => view["callId"].as_str().map(str::to_owned),
        };
        state
            .pending
            .iter()
            .filter_map(|item| Some((item.id.clone(), call(item)?)))
            .collect()
    }
    /// The session each waiting native card belongs to, by source, with when
    /// its first card came. A native channel can ask before its client's log
    /// names the session.
    pub fn native_sessions(&self) -> HashMap<(String, String), i64> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut result: HashMap<(String, String), i64> = HashMap::new();
        for item in &state.pending {
            let Content::Native(view) = &item.content else {
                continue;
            };
            if let (Some(source), Some(session)) =
                (view["source"].as_str(), view["sessionId"].as_str())
            {
                let at = result
                    .entry((source.into(), session.into()))
                    .or_insert(item.created_ms);
                *at = (*at).min(item.created_ms);
            }
        }
        result
    }
    /// Small per-snapshot index: presentation never locks this owner once for
    /// every historical transcript, or clones tool payloads just to sort sessions.
    pub fn session_attention(&self) -> HashMap<String, HashMap<String, Vec<Value>>> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut result: HashMap<String, HashMap<String, Vec<Value>>> = HashMap::new();
        for item in &state.pending {
            let (source, session, kind) = match &item.content {
                Content::Hook(request) => (
                    request.source.as_str(),
                    request.session_id.as_str(),
                    request.kind.as_str(),
                ),
                Content::Native(view) => {
                    match (view["source"].as_str(), view["sessionId"].as_str()) {
                        (Some(source), Some(session)) => (
                            source,
                            session,
                            view["kind"].as_str().unwrap_or("permission"),
                        ),
                        _ => continue,
                    }
                }
            };
            result.entry(source.into()).or_default().entry(session.into()).or_default().push(
                serde_json::json!({"id":item.id,"sessionId":session,"kind":kind,"at":chrono::DateTime::<chrono::Utc>::from_timestamp_millis(item.created_ms).map(|v|v.to_rfc3339())})
            );
        }
        result
    }
    pub fn views(&self, now_ms: i64) -> Vec<Value> {
        self.expire(now_ms);
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .iter()
            .map(|item| match &item.content {
                Content::Hook(request) => request.view(&item.id, item.created_ms),
                Content::Native(view) => view.clone(),
            })
            .collect()
    }
}
