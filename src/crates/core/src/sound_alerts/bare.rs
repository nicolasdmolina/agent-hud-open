//! The names waits are decided under, and the ids their alerts are told
//! under (Agent HUD Fork, ticket 75).
//!
//! A wait its hooks reported is known by the time of that report, a time
//! of its own. A wait only a channel's waiting flag tells has none, and
//! borrows the time of the session's last event; a request or an ask its
//! channel names with no card goes by the request's or the ask's id. Each
//! such name (its *base*) can come back for a later wait, with nothing
//! written between. So each base has a count of the waits under it that
//! were seen to end, and a wait goes by its base and that count: the
//! count only goes up when a wait ends, so a wait that goes on keeps its
//! name, and the next one gets a name no wait had (ticket 66, round 4).
//!
//! The count is touched at every look that sees a wait under its base,
//! so it is never forgotten before a name it gave: a day after the last
//! wait under a base, the count and every name under it go together, and
//! the next wait starts again from the base. Counts are saved with the
//! decided names (`kept.rs`), so a relaunch changes nothing of this.
//!
//! The host plays an alert unless the snapshot before carried its id
//! (`host/src/system/sound.rs`, `fresh`), and the snapshot keeps the last
//! 32 alerts however old. A name decided anew after its day can be the id
//! of an alert still carried, so an alert's id is its name, or, where an
//! alert the snapshot carries has that id, its name with `~2`, `~3`, ...
//! The id is never read back; it only has to be one the host plays.
use super::{Tracker, KEPT_MS};
use serde_json::Value;
use std::collections::HashSet;

/// How the fork's earlier builds saved that a wait went by a name and was
/// seen to end: `<name>/over`, the name being the base and one `+` for
/// each earlier end. Read once, when the saved file is loaded.
const OVER: &str = "/over";

impl Tracker {
    /// The name the wait with no card of `session` goes by now, and the
    /// base it borrowed, if its time is not its own.
    pub(super) fn bare_name(
        &mut self,
        session: &Value,
        source: &str,
        id: &str,
        now: i64,
    ) -> (String, Option<String>) {
        let facts = &session["liveFacts"];
        let own = facts["attention"]["at"].as_str();
        let since = own
            .or(facts["observedAt"].as_str())
            .or(session["updatedAt"].as_str())
            .unwrap_or("");
        let base = format!("needs-you:{source}:{id}:{since}");
        match own {
            Some(_) => (base, None),
            None => (self.named(&base, now), Some(base)),
        }
    }

    /// The name a wait under `base` goes by: the base, after as many
    /// waits under it as were seen to end. The count is touched.
    pub(super) fn named(&mut self, base: &str, now: i64) -> String {
        match self.ends.get_mut(base) {
            Some((count, at)) => {
                *at = now;
                format!("{base}+{count}")
            }
            None => base.to_owned(),
        }
    }

    /// Touches the count of `base`, where it has one: a card goes by its
    /// plain name, and keeps the count of the waits under it alive.
    pub(super) fn touch(&mut self, base: &str, now: i64) {
        if let Some((_, at)) = self.ends.get_mut(base) {
            *at = now;
        }
    }

    /// A wait under `base` is seen to end: the next goes by the name after.
    pub(super) fn end(&mut self, base: Option<String>, now: i64) {
        let Some(base) = base else { return };
        let count = self.ends.entry(base).or_insert((0, now));
        *count = (count.0 + 1, now);
        self.changed = true;
    }

    /// A wait under `base` ends that, at the look `before`, went by the
    /// plain base with no card: the base is forgotten. A card of its
    /// request that comes afterwards is then another wait, and sounds; a
    /// card shown again after its own wait ended is still the same card.
    pub(super) fn spent(&mut self, before: Option<&super::Wait>, base: &str) {
        if before.is_some_and(|wait| wait.uncarded.iter().any(|name| name == base)) {
            self.changed |= self.decided.remove(base).is_some();
        }
    }

    /// The name each request its channel names for the wait goes by at
    /// this look, by the request's id: the name its card has, where it
    /// has one. A request the wait named at the look `before` and names
    /// no more is over (it was settled, or the channel lost sight of it),
    /// and whatever the channel names under that id afterwards is another
    /// wait (ticket 66, rounds 4 and 9).
    pub(super) fn request_names(
        &mut self,
        before: Option<&super::Wait>,
        named: &[String],
        session: &str,
        now: i64,
    ) -> Vec<(String, String)> {
        let base = |request: &String| format!("needs-you:{session}:{request}");
        for (request, _) in before.into_iter().flat_map(|wait| &wait.named) {
            if !named.contains(request) {
                self.spent(before, &base(request));
                self.end(Some(base(request)), now);
            }
        }
        let named = named.iter().map(|r| (r.clone(), base(r)));
        named.map(|(r, base)| (r, self.named(&base, now))).collect()
    }

    /// Forgets every name and count last among the inputs a day ago, but
    /// what the look at `last` had among them. Called at the end of a look
    /// with its own time, and at the start of the next with the time of the
    /// one before: after a sleep of over a day, what went on until then
    /// goes on, and what had ended is forgotten before it is decided by.
    pub(super) fn forget(&mut self, now: i64, last: i64) {
        let (decided, ends) = (self.decided.len(), self.ends.len());
        self.decided
            .retain(|_, at| now - *at < KEPT_MS || *at == last);
        self.ends
            .retain(|_, (_, at)| now - *at < KEPT_MS || *at == last);
        self.changed |= self.decided.len() != decided || self.ends.len() != ends;
    }

    /// The id the alert of `name` is told under: one the snapshot does
    /// not carry, so the host plays it.
    pub(super) fn unused(&self, name: String) -> String {
        let carried: HashSet<&str> = self
            .items
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();
        if !carried.contains(name.as_str()) {
            return name;
        }
        let ids = (2..).map(|n| format!("{name}~{n}"));
        ids.into_iter()
            .find(|id| !carried.contains(id.as_str()))
            .unwrap_or(name)
    }

    /// Takes the marks an earlier build saved (`<base>++/over`) as counts:
    /// the wait after them goes by a name none of them had.
    pub(super) fn adopt_marks(&mut self) {
        let marks: Vec<(String, i64)> = self
            .decided
            .iter()
            .filter_map(|(id, at)| Some((id.strip_suffix(OVER)?.to_owned(), *at)))
            .collect();
        for (name, at) in marks {
            self.decided.remove(&format!("{name}{OVER}"));
            let base = name.trim_end_matches('+');
            let count = (name.len() - base.len() + 1) as u32;
            let entry = self.ends.entry(base.to_owned()).or_insert((0, at));
            *entry = (entry.0.max(count), entry.1.max(at));
        }
    }
}
