//! How the refresh is paced and bounded, and what it says of a machine.
use crate::clients::hermes::far_history::Settings;
use agent_hud_agents::hermes::mirror::Reads;
use std::time::Duration;

/// How the refresh paces itself and how much it reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan {
    /// Whether ticks come by the clock. Tests tick by hand.
    pub clock: bool,
    /// From the machine being followed to its first tick.
    pub first: Duration,
    /// Between two ticks while a far session is in a turn, or a whole
    /// sweep is overdue.
    pub live: Duration,
    /// Between two ticks otherwise, however many sessions are listed.
    pub idle: Duration,
    /// How long a turn a poll found keeps the quick pace: the tick after
    /// a turn's end still comes at it, and the ones after that do not.
    pub worked: Duration,
    /// The most reads of one home in one tick, four at least.
    pub reads: u32,
    /// The rows one page of fingerprints looks at.
    pub page: u32,
    /// The most sessions one read asks for.
    pub batch: usize,
    /// A whole sweep must finish within this.
    pub sweep_within: Duration,
    /// How far back the recent sweep looks.
    pub recent: Duration,
    /// How one read is run.
    pub reader: Settings,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            clock: true,
            first: Duration::from_secs(10),
            live: Duration::from_secs(60),
            idle: Duration::from_secs(300),
            worked: FRESH,
            reads: 8,
            page: 500,
            batch: 25,
            sweep_within: Duration::from_secs(24 * 3600),
            recent: Duration::from_secs(7 * 24 * 3600),
            reader: Settings::default(),
        }
    }
}

impl Plan {
    /// From a tick that came to `ticked` to the next. Sooner while a far
    /// session is in a turn (`working`), a whole sweep is overdue, or
    /// nothing could be read at all just now. Sessions that are listed
    /// and idle do not hurry it, and a machine with no reader
    /// (`without`) is asked at the slow pace whatever else holds.
    pub(super) fn wait(&self, ticked: Ticked, working: bool, without: bool) -> Duration {
        let soon = ticked.reads == 0 || ticked.overdue || working;
        match soon && !without {
            true => self.live,
            false => self.idle,
        }
    }
}

/// A live list older than this says nothing of what is live now; and,
/// as the plan's own `worked`, a session found in a turn longer ago than
/// this is not taken to be in one.
pub(super) const FRESH: Duration = Duration::from_secs(120);

/// What the machines list says of a machine's history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ages {
    /// When any of its profiles was last read at all.
    pub contact_ms: Option<i64>,
    /// When the oldest of its profiles' last whole sweeps finished. None
    /// until every profile that has a database was swept whole once.
    pub checked_ms: Option<i64>,
    /// Why the machine has no history reader, where it has none.
    pub without: Option<Without>,
    /// How many of its sessions the mirror would not take, which are not
    /// counted until it does (ticket 61). Known since this launch.
    pub refused: usize,
}

impl Ages {
    /// A machine's ages from those of its profiles' mirrors: the newest
    /// contact with any of them, and the oldest of their last whole
    /// sweeps, which is none while one of them was never swept whole.
    pub(super) fn of<'a>(
        profiles: impl Iterator<Item = &'a Reads> + Clone,
        without: Option<Without>,
    ) -> Self {
        let swept = profiles.clone().map(|reads| reads.sweep_done_ms);
        Self {
            contact_ms: profiles.filter_map(|reads| reads.contact_ms).max(),
            checked_ms: match swept.clone().all(|done| done.is_some()) {
                true => swept.flatten().min(),
                false => None,
            },
            without,
            refused: 0,
        }
    }
}

/// Why a machine has no history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Without {
    /// No `python3` there, or one without `sqlite3`.
    Python,
    /// Its SQLite cannot bound what it reads.
    SqliteOld,
}

/// What one tick found, for the wait before the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ticked {
    /// A whole sweep is still overdue on some profile.
    pub overdue: bool,
    /// How many far reads it made.
    pub reads: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quick_pace_is_for_work_under_way_and_the_slow_one_for_everything_else() {
        // The paces themselves: a minute, and five.
        let plan = Plan::default();
        let (quick, slow) = (Duration::from_secs(60), Duration::from_secs(300));
        assert_eq!((plan.live, plan.idle), (quick, slow));
        // A turn keeps the quick pace for two minutes past the last poll
        // that found it.
        assert_eq!(plan.worked, Duration::from_secs(120));
        let read = Ticked {
            overdue: false,
            reads: 3,
        };
        // Nothing in a turn, no sweep overdue: slow, whatever is listed.
        assert_eq!(plan.wait(read, false, false), slow);
        // A session in a turn, a sweep overdue, or nothing read: quick.
        assert_eq!(plan.wait(read, true, false), quick);
        let overdue = Ticked {
            overdue: true,
            ..read
        };
        assert_eq!(plan.wait(overdue, false, false), quick);
        assert_eq!(plan.wait(Ticked::default(), false, false), quick);
        // No reader there: slow, whatever else holds.
        for ticked in [read, overdue, Ticked::default()] {
            assert_eq!(plan.wait(ticked, true, true), slow);
        }
    }

    #[test]
    fn a_machine_was_checked_whole_when_the_last_of_its_profiles_was() {
        let reads = |contact_ms, sweep_done_ms| Reads {
            contact_ms,
            sweep_done_ms,
            ..Reads::default()
        };
        let none: [Reads; 0] = [];
        assert_eq!(Ages::of(none.iter(), None), Ages::default());
        // The newest contact, and the oldest whole check.
        let both = [reads(Some(900), Some(500)), reads(Some(700), Some(300))];
        let ages = Ages::of(both.iter(), None);
        assert_eq!((ages.contact_ms, ages.checked_ms), (Some(900), Some(300)));
        // One profile never swept whole: the machine was not either.
        let one = [reads(Some(900), Some(500)), reads(Some(950), None)];
        let ages = Ages::of(one.iter(), Some(Without::Python));
        assert_eq!((ages.contact_ms, ages.checked_ms), (Some(950), None));
        assert_eq!(ages.without, Some(Without::Python));
    }
}
