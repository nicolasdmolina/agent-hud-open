//! Cards for sessions no log row holds yet. A client's hooks or its native
//! channel can name a session before the HUD reads its log (a Hermes
//! question in a session just started), and only log rows took their cards.
//! A row only hooks made takes its cards as a log row does, and a native card
//! whose session has no row at all gets one; either way the session waits,
//! and sounds, at the pass the card's wake asks for. Once the log names the
//! session, its own row takes over under the same id.
use super::*;
use crate::naming::session_title::{self, TitleEvidence};

impl crate::AgentHudCore {
    /// Lays the waiting cards over the rows only hooks made.
    pub(super) fn carded_hook_rows(&self, rows: &mut [Value], now: i64) {
        if rows.is_empty() || self.requests.session_attention().is_empty() {
            return;
        }
        let presentation = self.presentation(now);
        for row in rows {
            presentation.cards(row);
        }
    }

    /// A waiting row for each native card whose session `sessions` lacks,
    /// for every native channel alike.
    pub(super) fn card_only_rows(&self, sessions: &[Value], now: i64) -> Vec<Value> {
        let carded = self.requests.native_sessions();
        if carded.is_empty() {
            return Vec::new();
        }
        let mut listed: HashSet<(&str, &str)> = sessions
            .iter()
            .filter_map(|row| Some((row["source"].as_str()?, row["id"].as_str()?)))
            .collect();
        // A session its log hides on purpose stays hidden: gone, archived,
        // internal, or a Claude project's worker.
        listed.extend(
            self.logs
                .values()
                .filter(|log| {
                    log.gone
                        || log.archived
                        || log.live.is_internal
                        || self.is_claude_project_worker(log)
                })
                .map(|log| (log.source.as_str(), log.id.as_str())),
        );
        let presentation = self.presentation(now);
        let mut rows: Vec<Value> = carded
            .into_iter()
            .filter(|((source, id), _)| !listed.contains(&(source.as_str(), id.as_str())))
            .filter(|((source, _), _)| self.enabled.get(source).copied().unwrap_or(true))
            .map(|((source, id), at_ms)| {
                let title = session_title::display(&source, &id, &TitleEvidence::default(), None, None);
                let at = crate::iso(at_ms);
                let mut row = json!({"id":id,"source":source,"title":title,"model":"Unknown","cwd":null,"updatedAt":at,"lastMessageAt":at,"state":"idle","tokens":0,"cost":null,"contextPercent":null,"logPath":null,"parentId":null,"usageStatus":"unavailable","evidence":"native-card"});
                presentation.overlay(&mut row, Some(at_ms), None, &Default::default());
                row
            })
            .collect();
        rows.sort_unstable_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        rows
    }
}
