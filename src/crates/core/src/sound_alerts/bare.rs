//! The name of a wait with no card. A wait its hooks reported is known by
//! the time of that report, a time of its own. A wait only a channel's
//! waiting flag tells has none, and borrows the time of the session's last
//! event: every wait the session has while nothing is written would go by
//! that one name. The host plays an alert once by its name, so a second
//! wait under it would be decided again and never heard. A borrowed name
//! is therefore marked as over once its wait is seen to end, and the next
//! wait goes by the name after it (ticket 66, round 4).
//!
//! The mark outlives the day while an alert the snapshot carries still
//! goes by the name it marks (Agent HUD Fork, ticket 75). The host plays
//! an id once while its last snapshot carries it, and the snapshot keeps
//! the last 32 alerts however old: a mark forgotten after a day would
//! give a later wait the name of an alert still there, and it would be
//! silent. So few marks are kept that way: one per alert at most.
use super::{Tracker, Wait, KEPT_MS};
use serde_json::Value;
use std::collections::HashSet;

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
        let name = format!("needs-you:{source}:{id}:{since}");
        match own {
            Some(_) => (name, false),
            // Past every wait that went by this time and was seen to end.
            None => (self.past(name), true),
        }
    }

    /// `name`, or the name after it, past every wait that went by it and
    /// was seen to end.
    pub(super) fn past(&self, mut name: String) -> String {
        while self.decided.contains_key(&format!("{name}{OVER}")) {
            name.push('+');
        }
        name
    }

    /// The name each request its channel names for the wait goes by at
    /// this look, by the request's id: the name its card has, where it has
    /// one. A request the wait named at the look `before` and names no
    /// more is over (it was settled, or the channel lost sight of it), so
    /// its name is marked, and whatever the channel names under that id
    /// afterwards is another wait and goes by the name after it. A wait
    /// seen to end and to begin again sounds again, for a request with no
    /// card as for a wait that names none (ticket 66, rounds 4 and 9).
    pub(super) fn request_names(
        &mut self,
        before: Option<&Wait>,
        named: &[String],
        session: &str,
        now: i64,
    ) -> Vec<(String, String)> {
        for (request, name) in before.into_iter().flat_map(|wait| &wait.named) {
            if !named.contains(request) {
                self.over(Some(name.clone()), now);
            }
        }
        let name = |request: &String| self.past(format!("needs-you:{session}:{request}"));
        named.iter().map(|r| (r.clone(), name(r))).collect()
    }

    /// Marks the name a wait borrowed as over, where it had one that is
    /// still remembered or that an alert the snapshot carries goes by: one
    /// about to be forgotten that no alert goes by needs no mark. A wait
    /// seen to end only at a look over a day after the last one is marked.
    pub(super) fn over(&mut self, name: Option<String>, now: i64) {
        let Some(name) = name else { return };
        let remembered = self
            .decided
            .get(&name)
            .is_some_and(|at| now - *at < KEPT_MS);
        if remembered || self.carried().contains(name.as_str()) {
            self.first(&format!("{name}{OVER}"), now);
        }
    }

    /// Forgets every identity last among the inputs a day ago, but the
    /// mark that a name is over while an alert the snapshot carries goes
    /// by that name.
    pub(super) fn forget(&mut self, now: i64) {
        let carried: HashSet<String> = self.carried().into_iter().map(str::to_owned).collect();
        let decided = self.decided.len();
        self.decided.retain(|id, at| {
            now - *at < KEPT_MS || id.strip_suffix(OVER).is_some_and(|n| carried.contains(n))
        });
        self.changed |= self.decided.len() != decided;
    }

    /// The ids of the alerts the snapshot carries.
    fn carried(&self) -> HashSet<&str> {
        self.items
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect()
    }
}
