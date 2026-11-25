//! One look of the sound decision, as the snapshot takes it: what the
//! decision is handed beside the rows, and what core is told afterwards.
use super::{Alert, Heard, Tracker, FINISHED_SOUND, KEPT_ITEMS, NEEDS_YOU_SOUND};
use crate::iso;
use crate::AgentHudCore;
use serde_json::json;
use serde_json::Value;

impl Tracker {
    /// Adds this look's alerts to the list the snapshot carries, each under
    /// an id it does not carry (`bare.rs`), and gives the list. With sound
    /// off the list is emptied.
    pub(super) fn tell(&mut self, alerts: Vec<Alert>, settings: &Value, now: i64) -> Value {
        // What waits at the first look was there before the app: noted only.
        let primed = std::mem::replace(&mut self.primed, true);
        if settings["sound"] != true {
            self.items.clear();
        } else {
            let mut added = 0;
            for alert in alerts {
                if alert.kind == "needs-you" && !primed {
                    continue;
                }
                let (key, default) = match alert.kind {
                    "needs-you" => ("soundNeedsYou", NEEDS_YOU_SOUND),
                    _ => ("soundFinished", FINISHED_SOUND),
                };
                added += 1;
                let id = self.unused(alert.id);
                self.revision = self.revision.wrapping_add(1);
                self.items.push_back(json!({"id":id,"kind":alert.kind,"source":alert.source,"sessionId":alert.session,"sound":settings[key].as_str().unwrap_or(default),"at":iso(now)}));
            }
            while self.items.len() > KEPT_ITEMS + added {
                self.items.pop_front();
            }
        }
        json!({"revision":self.revision,"items":self.items})
    }

    /// The approved requests whose wait a look saw end, once.
    pub(crate) fn take_ended(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.ended)
    }
}

impl AgentHudCore {
    /// This look's alerts, with what the rows no longer show: what cards
    /// hid, each card's call, the requests core holds as approved now, and the
    /// requests each session's wait is for. Core is then told which
    /// approved requests' waits the look saw end, and only those.
    pub(crate) fn sound_look(&mut self, island: &Value, sessions: &[Value], now: i64) -> Value {
        let heard = Heard {
            sightings: self.listed_rows.as_ref().map(|(_, rows)| &rows.heard),
            calls: self.requests.calls(),
            approved: self.requests.auto_approved(now),
            waits: self.requests.native_wait_requests(),
            asks: self.requests.native_wait_asks(),
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
