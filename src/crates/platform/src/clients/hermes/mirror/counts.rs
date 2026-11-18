//! What a machine's sessions have used so far, as the writer's thread
//! keeps it between a poll and a commit: the samples that wait for
//! their time, and the most each mirror is known to say of each
//! session, so that a poll whose counts did not move holds nothing back
//! to write. Only the writer's own thread is here.
use agent_hud_agents::hermes::gateway::{Profile, Sample};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Counts {
    /// The samples that wait to be written: for each profile the route
    /// number they were read under, and each session's newest with when
    /// it was read.
    waiting: HashMap<Profile, (u64, HashMap<String, (Sample, i64)>)>,
    /// The most each mirror is known to say each session has used, so a
    /// poll whose counts did not move holds nothing back to write.
    says: HashMap<Profile, HashMap<String, u64>>,
    /// When the machine's counts were last committed.
    counted: Option<Instant>,
}

impl Counts {
    /// Nothing is known of the mirror of `profile`, and nothing waits
    /// for it.
    pub(super) fn forget(&mut self, profile: &Profile) {
        self.waiting.remove(profile);
        self.says.remove(profile);
    }

    /// A poll's samples of `profile`, read at `read_ms` under the route
    /// number `route`, wait to be written where they say more than the
    /// mirror is known to. Ones that waited from another route number
    /// are dropped: `route` is the machine's now.
    pub(super) fn sample(
        &mut self,
        profile: &Profile,
        samples: Vec<(String, Sample)>,
        read_ms: i64,
        route: u64,
    ) {
        let other = |(under, _): &(u64, _)| *under != route;
        if self.waiting.get(profile).is_some_and(other) {
            self.waiting.remove(profile);
        }
        let says = self.says.get(profile);
        let more = |(id, sample): &(String, Sample)| {
            sample.total() > says.and_then(|says| says.get(id)).copied().unwrap_or(0)
        };
        let news: Vec<(String, Sample)> = samples.into_iter().filter(more).collect();
        if news.is_empty() {
            return;
        }
        let fresh = || (route, HashMap::new());
        let (_, waiting) = self.waiting.entry(profile.clone()).or_insert_with(fresh);
        for (id, sample) in news {
            let waits = waiting.get(&id);
            if waits.is_none_or(|(held, _)| sample.total() > held.total()) {
                waiting.insert(id, (sample, read_ms));
            }
        }
    }

    /// When the samples that wait may be written: `space` after counts
    /// were last committed. None where none waits.
    pub(super) fn due(&self, space: Duration) -> Option<Instant> {
        if self.waiting.is_empty() {
            return None;
        }
        Some(self.counted.map_or_else(Instant::now, |at| at + space))
    }

    /// The profiles that have samples waiting, in order.
    pub(super) fn waiting(&self) -> Vec<Profile> {
        let mut profiles: Vec<Profile> = self.waiting.keys().cloned().collect();
        profiles.sort();
        profiles
    }

    /// The samples of `profile` that waited, which wait no more, with
    /// the route number they were read under.
    pub(super) fn take(&mut self, profile: &Profile) -> Option<(u64, Vec<(String, Sample, i64)>)> {
        let (route, waited) = self.waiting.remove(profile)?;
        let mut samples: Vec<(String, Sample, i64)> = (waited.into_iter())
            .map(|(id, (sample, read_ms))| (id, sample, read_ms))
            .collect();
        samples.sort_by(|a, b| a.0.cmp(&b.0));
        Some((route, samples))
    }

    /// The mirror of `profile` says at least this much of each session.
    pub(super) fn said(&mut self, profile: &Profile, samples: &[(String, Sample, i64)]) {
        let says = self.says.entry(profile.clone()).or_default();
        for (id, sample, _) in samples {
            let most = says.entry(id.clone()).or_default();
            *most = (*most).max(sample.total());
        }
    }

    /// The machine's counts were committed now.
    pub(super) fn counted(&mut self) {
        self.counted = Some(Instant::now());
    }
}
