//! What waits on the user, as the HUD and the hosts read it: the cards, the
//! sessions they belong to, and when they next change.
use super::{Content, Requests};
use serde_json::Value;
use std::{collections::HashMap, sync::PoisonError};

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
            // When each reader's reading lapses.
            .chain(state.native.deadlines())
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
