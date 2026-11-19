//! The sessions the Hermes desktop app hosts on the machine (ticket 39;
//! spec "Phase 2 design", section 15): the desktop app holds them, as the
//! refresh last read its far `runtime/active_sessions.json`, and no live
//! list a poll gave since a moment says so of them. Such a session runs in
//! the desktop app's own backend, which no gateway lists or takes an
//! answer for: it is listed from its stored row and gets no card, and the
//! machines list says its questions are answered in Hermes desktop.
//!
//! What is listed decides, not the held mark: a session both held and
//! listed live is the gateway's, with its cards. Each profile is its own:
//! a session's id names it only within its profile.
use super::{history::Live, Writer};
use agent_hud_agents::hermes::gateway::Profile;
use std::collections::BTreeSet;

impl Live {
    /// The refresh read that the desktop app holds `ids` in `profile`.
    fn hold(&mut self, profile: Profile, ids: Vec<String>) {
        let ids: BTreeSet<String> = ids.into_iter().collect();
        match ids.is_empty() {
            true => self.held.remove(&profile),
            false => self.held.insert(profile, ids),
        };
    }

    /// The held sessions that no live list read since `fresh_ms` lists. A
    /// profile whose last list is older says nothing of what is live, so
    /// its held sessions are in no live list.
    fn hosted(&self, fresh_ms: i64) -> Vec<(Profile, String)> {
        let mut hosted = Vec::new();
        for (profile, held) in &self.held {
            let listed = (self.of.get(profile))
                .filter(|listing| listing.read_ms >= fresh_ms)
                .map(|listing| &listing.now);
            let live = |id: &&String| listed.is_some_and(|now| now.contains(*id));
            hosted.extend(
                held.iter()
                    .filter(|id| !live(id))
                    .map(|id| (profile.clone(), id.clone())),
            );
        }
        hosted.sort();
        hosted
    }
}

impl Writer {
    /// The refresh's word of what the desktop app holds in `profile`,
    /// once the mirror took it.
    pub(super) fn holds(&self, profile: Profile, ids: Vec<String>) {
        let mut state = self.state();
        if !state.over {
            state.live.hold(profile, ids);
        }
    }

    /// The sessions the desktop app hosts on the machine now, by profile
    /// and Hermes's id; none once the writer has stopped.
    pub(crate) fn desktop_hosted(&self, fresh_ms: i64) -> Vec<(Profile, String)> {
        let state = self.state();
        match state.over {
            true => Vec::new(),
            false => state.live.hosted(fresh_ms),
        }
    }
}
