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
//! silent. A wait that goes on keeps the name it was decided under, and
//! a mark stays while the name after it is remembered: a wait that went
//! on for over a day, or through a relaunch, never falls back to an older
//! name, and the wait after it is never given one decided for it (round
//! 2). Few marks are kept past their day: one per carried alert, and one
//! per name still remembered that came after a mark.
use super::{Tracker, Wait, KEPT_MS};
use serde_json::Value;
use std::collections::HashSet;

/// What marks a borrowed name as that of a wait seen to end.
const OVER: &str = "/over";

impl Tracker {
    /// The name the wait with no card of `session` goes by now, and whether
    /// its time is borrowed.
    pub(super) fn bare_name(
        &self,
        session: &Value,
        source: &str,
        id: &str,
        before: Option<&Wait>,
    ) -> (String, bool) {
        let facts = &session["liveFacts"];
        let own = facts["attention"]["at"].as_str();
        let since = own
            .or(facts["observedAt"].as_str())
            .or(session["updatedAt"].as_str())
            .unwrap_or("");
        let name = format!("needs-you:{source}:{id}:{since}");
        match own {
            Some(_) => (name, false),
            // The name it was decided under while it goes on; otherwise
            // past every wait that went by this time and was seen to end.
            None => {
                let lent = before.and_then(|wait| wait.lent.as_ref());
                let after = |lent: &&String| {
                    lent.strip_prefix(name.as_str())
                        .is_some_and(|rest| rest.bytes().all(|b| b == b'+'))
                };
                (
                    lent.filter(after)
                        .cloned()
                        .unwrap_or_else(|| self.past(name)),
                    true,
                )
            }
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

    /// The name a card goes by: `name`, unless it was forgotten while the
    /// mark that its wait is over stayed (its alert is still carried), and
    /// then the name after it. A card a day after its request waited with
    /// no card is then heard.
    pub(super) fn current(&self, mut name: String) -> String {
        while !self.decided.contains_key(&name)
            && self.decided.contains_key(&format!("{name}{OVER}"))
        {
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
        // A request the wait named before keeps its name while it waits.
        let kept = |request: &String| {
            let mut was = before.into_iter().flat_map(|wait| &wait.named);
            was.find(|(r, _)| r == request)
                .map(|(_, name)| name.clone())
        };
        let name = |request: &String| {
            kept(request).unwrap_or_else(|| self.past(format!("needs-you:{session}:{request}")))
        };
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
    /// by that name, or while the name after it is remembered.
    pub(super) fn forget(&mut self, now: i64) {
        let mut kept: HashSet<String> = self.carried().into_iter().map(str::to_owned).collect();
        let after = self.decided.keys().filter_map(|id| id.strip_suffix('+'));
        kept.extend(after.map(str::to_owned).collect::<Vec<_>>());
        let decided = self.decided.len();
        self.decided.retain(|id, at| {
            now - *at < KEPT_MS || id.strip_suffix(OVER).is_some_and(|n| kept.contains(n))
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
