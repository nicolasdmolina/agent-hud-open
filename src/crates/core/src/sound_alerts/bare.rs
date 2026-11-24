//! The name of a wait with no card. A wait its hooks reported is known by
//! the time of that report, a time of its own. A wait only a channel's
//! waiting flag tells has none, and borrows the time of the session's last
//! event: every wait the session has while nothing is written would go by
//! that one name. The host plays an alert once by its name, so a second
//! wait under it would be decided again and never heard. A borrowed name
//! is therefore marked as over once its wait is seen to end, and the next
//! wait goes by the name after it (ticket 66, round 4).
use super::{Tracker, KEPT_MS};
use serde_json::Value;

/// What marks a borrowed name as that of a wait seen to end.
const OVER: &str = "/over";

impl Tracker {
    /// The name the wait with no card of `session` goes by now, and whether
    /// its time is borrowed.
    pub(super) fn bare_name(&self, session: &Value, source: &str, id: &str) -> (String, bool) {
        let facts = &session["liveFacts"];
        let own = facts["attention"]["at"].as_str();
        let since = own
            .or(facts["observedAt"].as_str())
            .or(session["updatedAt"].as_str())
            .unwrap_or("");
        let mut name = format!("needs-you:{source}:{id}:{since}");
        if own.is_none() {
            // Past every wait that went by this time and was seen to end.
            while self.decided.contains_key(&format!("{name}{OVER}")) {
                name.push('+');
            }
        }
        (name, own.is_none())
    }

    /// Marks the name a wait borrowed as over, where it had one that is
    /// still remembered: one about to be forgotten needs no mark.
    pub(super) fn over(&mut self, name: Option<String>, now: i64) {
        let Some(name) = name else { return };
        if self
            .decided
            .get(&name)
            .is_some_and(|at| now - *at < KEPT_MS)
        {
            self.first(&format!("{name}{OVER}"), now);
        }
    }
}
