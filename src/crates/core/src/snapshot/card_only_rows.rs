//! Sessions only a waiting native card knows yet. A native channel can ask
//! before its client's log names the session (a Hermes question in a session
//! just started), and the log is read on a later pass. The card's row waits,
//! and sounds, as soon as the card is registered; once the log names the
//! session, its own row takes over under the same id.
use super::*;
use crate::naming::session_title::{self, TitleEvidence};

impl crate::AgentHudCore {
    /// A waiting row for each native card whose session `sessions` lacks.
    pub(super) fn card_only_rows(&self, sessions: &[Value], now: i64) -> Vec<Value> {
        let carded = self.requests.native_sessions();
        if carded.is_empty() {
            return Vec::new();
        }
        let listed: HashSet<(&str, &str)> = sessions
            .iter()
            .filter_map(|row| Some((row["source"].as_str()?, row["id"].as_str()?)))
            .collect();
        let presentation = self.presentation(now);
        let mut rows: Vec<Value> = carded
            .into_iter()
            .filter(|((source, id), _)| !listed.contains(&(source.as_str(), id.as_str())))
            .filter(|((source, _), _)| self.enabled.get(source).copied().unwrap_or(true))
            .map(|((source, id), at_ms)| {
                let title = session_title::display(&source, &id, &TitleEvidence::default(), None, None);
                let at = crate::iso(at_ms);
                let mut row = json!({"id":id,"source":source,"title":title,"model":"Unknown","cwd":null,"updatedAt":at,"lastMessageAt":at,"state":"idle","tokens":0,"cost":null,"contextPercent":null,"logPath":null,"parentId":null,"usageStatus":"unavailable","evidence":"native-card"});
                // Waiting, with the card among its requests.
                presentation.overlay(&mut row, Some(at_ms), None, &Default::default());
                row
            })
            .collect();
        rows.sort_unstable_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        rows
    }
}
