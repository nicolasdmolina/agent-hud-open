//! A far session's token count while it runs, for showing and for
//! nothing else. A far machine's usage reaches the ledger from its own
//! database alone; until that is read, a session that works would show
//! no tokens. So a poll of a far gateway asks `session.stats` of each
//! session that works, and once more of one that started or worked at
//! the poll before and is listed as something else now, since a turn's
//! last call adds to the count as it ends. A session that only starts
//! is not asked: its agent has not begun, and one can be listed so for
//! days. The call reads the counters of the session's agent and
//! subscribes to nothing (`gateway/session_usage.py`, `stats`).
//!
//! Hermes answers zeros where it holds no agent for the session, as it
//! does for a scheduled job's: that is no count, and gives no sample.
//! What is given is the samples of the newest poll that counted, with
//! the live list they came with (`listing.rs`). No card is decided by a
//! sample, and a call that fails only leaves a session without one
//! until the next poll.
//!
//! A gateway of the machine Agent HUD runs on is never asked: its
//! database is read where it is.
use super::effect::Status;
use super::identity::{Profile, SessionRef};
use super::machine::Machine;
use super::poll::{Poll, Step};
use super::state::Gateway;
use serde_json::Value;
use std::collections::BTreeMap;

/// The four counts of a session so far, as its agent has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Sample {
    /// What is shown for it.
    pub fn total(&self) -> u64 {
        let all = [self.input, self.output, self.cache_read, self.cache_write];
        all.iter()
            .fold(0u64, |sum, count| sum.saturating_add(*count))
    }

    /// What a reply to `session.stats` counts; None where it counts
    /// nothing, or holds a count that is no whole number.
    pub(super) fn of(result: &Value) -> Option<Self> {
        let count = |key: &str| match &result[key] {
            Value::Null => Some(0),
            said => said.as_u64(),
        };
        let sample = Self {
            input: count("input_tokens")?,
            output: count("output_tokens")?,
            cache_read: count("cache_read_tokens")?,
            cache_write: count("cache_write_tokens")?,
        };
        (sample.total() > 0).then_some(sample)
    }
}

/// The samples one poll of a far gateway takes.
#[derive(Debug, Default)]
pub(super) struct Sampling {
    /// The sessions the poll before listed as starting or working, each
    /// with which of the two.
    before: BTreeMap<SessionRef, Status>,
    /// What each session asked answered; None where it gave no sample.
    taken: BTreeMap<SessionRef, Option<Sample>>,
}

impl Gateway {
    /// `poll`, which also takes samples where this gateway is a far
    /// machine's.
    pub(super) fn sampling(&self, mut poll: Poll) -> Poll {
        if matches!(self.id.machine(), Machine::Far(_)) {
            let ran = |status: &Status| matches!(status, Status::Starting | Status::Working);
            let before = self.listed.iter().filter(|(_, status)| ran(status));
            poll.sampling = Some(Box::new(Sampling {
                before: before.map(|(at, status)| (at.clone(), *status)).collect(),
                taken: BTreeMap::new(),
            }));
        }
        poll
    }
}

impl Poll {
    /// The sessions still to ask `session.stats` of.
    pub(super) fn unsampled(&self) -> impl Iterator<Item = Step> + '_ {
        let sampling = self.sampling.as_deref().into_iter();
        sampling.flat_map(move |sampling| {
            let wanted = move |(at, look): (&SessionRef, &super::poll::Look)| {
                let works = look.status == Status::Working;
                // Started or worked, and is listed as something else now.
                let moved = (sampling.before.get(at)).is_some_and(|was| *was != look.status);
                let asked = sampling.taken.contains_key(at);
                ((works || moved) && !asked).then(|| Step::Stats(at.clone()))
            };
            self.sessions.iter().filter_map(wanted)
        })
    }

    /// The reply to a `session.stats` of `at`, or None where it failed.
    pub(super) fn sampled(&mut self, at: &SessionRef, result: Option<&Value>) {
        if let Some(sampling) = &mut self.sampling {
            let sample = result.and_then(Sample::of);
            sampling.taken.entry(at.clone()).or_insert(sample);
        }
    }

    /// The samples taken, profile by profile, by Hermes's session id.
    pub(super) fn samples(&self) -> BTreeMap<Profile, Vec<(String, Sample)>> {
        let mut all: BTreeMap<Profile, Vec<(String, Sample)>> = BTreeMap::new();
        let sampling = self.sampling.as_deref().into_iter();
        let taken = sampling.flat_map(|sampling| &sampling.taken);
        for (at, sample) in taken {
            if let Some(sample) = sample {
                let of = all.entry(at.profile.clone()).or_default();
                of.push((at.id().to_owned(), *sample));
            }
        }
        all
    }
}
