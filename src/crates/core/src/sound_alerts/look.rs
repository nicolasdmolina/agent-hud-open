//! One look of the sound decision, as the snapshot takes it: what the
//! decision is handed beside the rows, and what core is told afterwards.
use super::{Heard, Tracker};
use crate::AgentHudCore;
use serde_json::Value;

impl Tracker {
    /// The approved requests whose wait a look saw end, once.
    pub(crate) fn take_ended(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.ended)
    }
}

impl AgentHudCore {
    /// This look's alerts, with what the rows no longer show: what cards
    /// hid, each card's call, the requests allowed with no card, and the
    /// requests each session's wait is for. Core is then told which
    /// approved requests' waits the look saw end, and only those.
    pub(crate) fn sound_look(&mut self, island: &Value, sessions: &[Value], now: i64) -> Value {
        let heard = Heard {
            sightings: self.listed_rows.as_ref().map(|(_, rows)| &rows.heard),
            calls: self.requests.calls(),
            approved: self.requests.auto_approved(now),
            unanswered: self.requests.auto_unanswered(now),
            waits: self.requests.native_wait_requests(),
        };
        let alerts = self
            .sound_alerts
            .update(island, sessions, &heard, &self.settings, now);
        for (session, request) in self.sound_alerts.take_ended() {
            self.requests.auto_approved_ended(&session, &[request]);
        }
        alerts
    }
}
