//! What a machine's sessions have used so far, as the writer's thread
//! keeps it between a poll and a commit: the samples that wait for
//! their time, and the most each mirror is known to say of each
//! session, so that a poll whose counts did not move holds nothing back
//! to write. Only the writer's own thread is here.
//!
//! A count can reach the writer before the list that gives its session
//! a row (ticket 64): `hud_live` keeps no count of a session without
//! one. Such a count is held (`rowless`) and waits to be written again
//! once a list of its profile names its session; it is not taken for
//! what the mirror says. A newer list that does not name it, or one
//! read under another route number, drops it.
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
    /// The samples written while their session had no row, each with
    /// when it was read, and the route number they were read under.
    rowless: HashMap<Profile, (u64, HashMap<String, (Sample, i64)>)>,
    /// When the machine's counts were last committed.
    counted: Option<Instant>,
}

impl Counts {
    /// Nothing is known of the mirror of `profile`, and nothing waits
    /// for it. A count held for want of its row stays held: a mirror
    /// that is not there yet is forgotten too, and its first list is
    /// the one that gives the count its row.
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

    /// The `samples` of `profile`, read under `route`, were written while
    /// their sessions had no row: they are held until a list names them.
    pub(super) fn rowless(
        &mut self,
        profile: &Profile,
        route: u64,
        samples: Vec<(String, Sample, i64)>,
    ) {
        if samples.is_empty() {
            return;
        }
        let fresh = || (route, HashMap::new());
        let (under, held) = self.rowless.entry(profile.clone()).or_insert_with(fresh);
        if *under != route {
            (*under, *held) = fresh();
        }
        for (id, sample, read_ms) in samples {
            let more = |(kept, _): &(Sample, i64)| sample.total() > kept.total();
            if held.get(&id).is_none_or(more) {
                held.insert(id, (sample, read_ms));
            }
        }
    }

    /// A list of `profile` read at `read_ms` under `route` names
    /// `listed`: the held samples of those sessions wait to be written
    /// again, now that the list gives each its row. One of a session it
    /// does not name stays held where it was read after the list, and
    /// is dropped where it was read before; all are dropped where they
    /// were read under another route number.
    pub(super) fn replay(&mut self, profile: &Profile, listed: &[&str], read_ms: i64, route: u64) {
        let Some((under, held)) = self.rowless.remove(profile) else {
            return;
        };
        if under != route {
            return;
        }
        let mut left = HashMap::new();
        for (id, (sample, at)) in held {
            if listed.contains(&id.as_str()) {
                let fresh = || (route, HashMap::new());
                let (_, waiting) = self.waiting.entry(profile.clone()).or_insert_with(fresh);
                let waits = waiting.get(&id);
                if waits.is_none_or(|(kept, _)| sample.total() > kept.total()) {
                    waiting.insert(id, (sample, at));
                }
            } else if at > read_ms {
                left.insert(id, (sample, at));
            }
        }
        if !left.is_empty() {
            self.rowless.insert(profile.clone(), (route, left));
        }
    }
}
