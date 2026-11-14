//! What the polls of one connection cost, said in the log now and then.
use super::{polls, Shared};
use std::time::Duration;

/// What the polls cost, said in the log now and then: how many ran, how
/// long they took, how many calls and snapshots they made.
#[derive(Default)]
pub(super) struct Told {
    polls: u32,
    calls: usize,
    snapshots: usize,
    total: Duration,
    longest: Duration,
}

impl Told {
    /// How many polls one log line sums up: a minute of a busy gateway.
    const EVERY: u32 = 60;
    /// A poll this slow is said at once.
    const SLOW: Duration = Duration::from_millis(500);

    pub(super) fn add(&mut self, shared: &Shared, ran: &polls::Ran) {
        self.polls += 1;
        self.calls += ran.calls;
        self.snapshots += ran.snapshots;
        self.total += ran.took;
        self.longest = self.longest.max(ran.took);
        if ran.took >= Self::SLOW {
            shared.log.line(&format!(
                "slow poll ms={} calls={} snapshots={}",
                ran.took.as_millis(),
                ran.calls,
                ran.snapshots
            ));
        }
        if self.polls >= Self::EVERY {
            shared.log.line(&format!(
                "polls n={} mean_ms={} max_ms={} calls={} snapshots={}",
                self.polls,
                (self.total / self.polls).as_millis(),
                self.longest.as_millis(),
                self.calls,
                self.snapshots
            ));
            *self = Self::default();
        }
    }
}
