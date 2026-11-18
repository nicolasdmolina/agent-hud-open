//! A far session's token count while it runs, for showing and for
//! nothing else. A far machine's usage reaches the ledger from its own
//! database alone; until that is read, a session that works would show
//! no tokens. So a far gateway's sessions are asked `session.stats`,
//! which reads the counters of a session's agent and subscribes to
//! nothing (`gateway/session_usage.py`, `stats`).
//!
//! **Apart from the polls.** The calls are no step of a poll. They are
//! a round of their own (`Counting`), begun after a poll and asked on a
//! socket of its own, so no poll and no card waits for a count however
//! long Hermes takes to say one.
//!
//! **Who is asked.** Each session the newest poll listed as working,
//! in every round. And one that is owed a last count (`Owed`), until a
//! call for it is answered:
//!
//! - one that started, worked or waited at one poll and is listed as
//!   something else at the next: a turn's last call adds to the count
//!   as it ends;
//! - one a turn was read to start or end in (`turns.rs`), which is how
//!   a turn that ran wholly between two polls is known of at all.
//!
//! A call that fails leaves its session owed, and it is asked again in
//! the next round, `TRIES` rounds at most: its row then shows what it
//! showed until its history is read. A session that only starts is not
//! asked: its agent has not begun, and one can be listed so for days.
//!
//! Hermes answers zeros where it holds no agent for the session, as it
//! does for a scheduled job's: that is an answer, and no count, and
//! gives no sample. No card is decided by a sample.
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

/// How many rounds in a row a session that is owed a count is asked
/// again after a call for it failed.
pub(super) const TRIES: u32 = 5;

/// The sessions that are owed one more count.
#[derive(Debug, Default)]
pub(super) struct Owed {
    /// Each with the number it came to be owed under, and how many
    /// calls for it have failed since.
    sessions: BTreeMap<SessionRef, (u64, u32)>,
    /// Counts every time a session came to be owed.
    last: u64,
}

impl Owed {
    /// `at` is owed a count from now, whatever was asked of it before.
    pub(super) fn owe(&mut self, at: &SessionRef) {
        self.last += 1;
        self.sessions.insert(at.clone(), (self.last, 0));
    }

    /// Nothing is owed any more: the gateway is another one.
    pub(super) fn forget(&mut self) {
        self.sessions.clear();
    }
}

/// What a session asked in a round said.
#[derive(Debug)]
enum Said {
    /// Not yet.
    Nothing,
    /// The call failed, or was never answered.
    Failed,
    /// The reply, with the sample it gave where it counted anything.
    Answered(Option<Sample>),
}

/// One round of count calls: the sessions asked, and what each said.
/// Whoever runs it asks for `steps()`, gives each result to `answer`
/// (or says it `failed`), and hands the round to the model as
/// `Input::Counted`, having taken its `samples`.
#[derive(Debug)]
pub struct Counting {
    /// Each session asked, with the number it was owed under, where it
    /// was owed. A session's name holds its gateway's, so a round says
    /// nothing of another gateway's sessions.
    asked: BTreeMap<SessionRef, (Option<u64>, Said)>,
}

impl Counting {
    /// The calls still to be answered. They may go out at once.
    pub fn steps(&self) -> Vec<Step> {
        let unanswered = |(at, (_, said)): (&SessionRef, &(Option<u64>, Said))| {
            matches!(said, Said::Nothing).then(|| Step::Stats(at.clone()))
        };
        self.asked.iter().filter_map(unanswered).collect()
    }

    /// The `result` of the reply to `step`.
    pub fn answer(&mut self, step: &Step, result: &Value) {
        self.say(step, Said::Answered(Sample::of(result)));
    }

    /// `step` was refused or never answered.
    pub fn failed(&mut self, step: &Step) {
        self.say(step, Said::Failed);
    }

    fn say(&mut self, step: &Step, said: Said) {
        let Step::Stats(at) = step else {
            return;
        };
        if let Some((_, was @ Said::Nothing)) = self.asked.get_mut(at) {
            *was = said;
        }
    }

    /// The samples taken, profile by profile, by Hermes's session id.
    pub fn samples(&self) -> BTreeMap<Profile, Vec<(String, Sample)>> {
        let mut all: BTreeMap<Profile, Vec<(String, Sample)>> = BTreeMap::new();
        for (at, (_, said)) in &self.asked {
            if let Said::Answered(Some(sample)) = said {
                let of = all.entry(at.profile.clone()).or_default();
                of.push((at.id().to_owned(), *sample));
            }
        }
        all
    }
}

impl Gateway {
    /// Begins a round of count calls: every session the newest poll
    /// that counted listed as working, and every one owed a count. None
    /// where nobody is to be asked, and for a gateway of this machine.
    /// Only the sessions of profiles that poll listed are asked, as
    /// only they are current (`live.rs`): none while the gateway is not
    /// ready, nor before a poll has listed it since it last was.
    pub fn count(&self) -> Option<Counting> {
        if !matches!(self.id.machine(), Machine::Far(_)) {
            return None;
        }
        let current = |at: &SessionRef| self.told.fresh().contains(&at.profile);
        let works = (self.listed.iter())
            .filter(|(at, status)| **status == Status::Working && current(at))
            .map(|(at, _)| (at.clone(), (None, Said::Nothing)));
        let mut asked: BTreeMap<SessionRef, (Option<u64>, Said)> = works.collect();
        for (at, (under, _)) in &self.owed.sessions {
            if current(at) {
                asked.insert(at.clone(), (Some(*under), Said::Nothing));
            }
        }
        (!asked.is_empty()).then_some(Counting { asked })
    }

    /// A round of count calls came back. A session that was owed a
    /// count and answered is owed none; one whose call failed is asked
    /// again, `TRIES` rounds at most. One that came to be owed anew
    /// while the round was out stays owed: what it said was said before
    /// that.
    pub(super) fn counted(&mut self, counting: &Counting) {
        for (at, (under, said)) in &counting.asked {
            let owed = self.owed.sessions.get_mut(at);
            let Some((_, tries)) = owed.filter(|(now, _)| Some(*now) == *under) else {
                continue;
            };
            *tries += 1;
            if matches!(said, Said::Answered(_)) || *tries >= TRIES {
                self.owed.sessions.remove(at);
            }
        }
    }

    /// What `poll`, which counts, says of who is owed a count, before
    /// its list replaces the one before: a session that started, worked
    /// or waited and is listed as something else now. One that works
    /// now is asked in every round as it is.
    pub(super) fn owe_moved(&mut self, poll: &Poll) {
        let ran = |was: &Status| *was != Status::Idle;
        for (at, look) in &poll.sessions {
            let was = self.listed.get(at);
            let moved = was.is_some_and(|was| ran(was) && *was != look.status);
            if moved && look.status != Status::Working {
                self.owed.owe(at);
            }
        }
    }

    /// A session that left the live list is owed nothing: Hermes holds
    /// no agent for it to ask.
    pub(super) fn keep_owed(&mut self) {
        let listed = &self.listed;
        self.owed.sessions.retain(|at, _| listed.contains_key(at));
    }
}
