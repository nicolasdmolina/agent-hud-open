//! How the connection paces itself: how long it waits before dialling
//! again, between polls, and for an answer. Tests shorten it.
#[cfg(test)]
use agent_hud_agents::hermes::gateway::IDLE_POLL_MS;
use agent_hud_agents::hermes::gateway::QUIET_MS;
use std::time::Duration;

/// The waits of one connection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pace {
    /// The first wait before dialling again, doubled each time it fails.
    pub retry: Duration,
    /// The longest wait before dialling again while a gateway may come back.
    pub retry_limit: Duration,
    /// How long before asking a gateway without the live connection again:
    /// an update can add it.
    pub unsupported: Duration,
    /// The least time between the starts of two polls, however many ask.
    pub gap: Duration,
    /// The longest time between the starts of two polls while the gateway
    /// has a live session (on a far machine, one that is not quiet: the
    /// model's `lively`): a turn can begin and ask in any of them, and
    /// nothing tells of it but the poll. The model's own timer is slower
    /// while every session is idle; measured against a real gateway, that
    /// showed a question up to ten seconds late, or never when it was
    /// answered sooner elsewhere. Hermes's desktop app asks the same list
    /// every 1.5 s.
    pub watch: Duration,
    /// How long that pace is kept after a hint or a hook asked for a poll:
    /// they tell that something is about to happen, not what.
    pub warm: Duration,
    /// How long the control socket and one call may take.
    pub answer: Duration,
    /// How long one frame on a short socket (a resume, an answer) may take
    /// to write, all of it. A frame is small and the gateway is on this
    /// machine: one that does not take it in this long is not going to.
    pub write: Duration,
    /// How long one read of a session's events may take before that
    /// session is read apart from the polls (`apart.rs`): a poll is to be
    /// over within a second, whatever one session's events take to read.
    pub apart: Duration,
    /// How long a poll waits, all in all, for what it asks last: the
    /// reads of sessions nothing has changed in and the looks at idle
    /// ones (`waits.rs`). With the ten seconds between two polls of a far
    /// gateway where nothing moves, and the second a poll has for its
    /// own work, it keeps a card there within its thirteen.
    pub still: Duration,
    /// How long between two looks at every Hermes home of the machine
    /// (`discovery.rs`), the first as soon as the observer starts. None
    /// for no look by the clock: one is taken only when a follower asks.
    pub discover: Option<Duration>,
    /// How long nothing may change in a listed session before it stops
    /// hurrying the polls and the history (the model's `quiet.rs`).
    pub quiet: Duration,
    /// How a far machine's stored history is kept in step (`history.rs`).
    pub history: super::history::Plan,
    /// Whether the log says every poll and every hint, for looking at what
    /// a real gateway does. Never on in the app.
    pub trace: bool,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            retry: Duration::from_millis(250),
            retry_limit: Duration::from_secs(5),
            unsupported: Duration::from_secs(300),
            gap: Duration::from_millis(250),
            watch: Duration::from_millis(800),
            warm: Duration::from_secs(5),
            answer: Duration::from_secs(5),
            write: Duration::from_millis(500),
            apart: Duration::from_millis(250),
            still: Duration::from_secs(2),
            discover: Some(Duration::from_secs(10)),
            quiet: Duration::from_millis(QUIET_MS as u64),
            history: super::history::Plan::default(),
            trace: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_quiet_time_is_the_models_thirty_minutes() {
        assert_eq!(Pace::default().quiet, Duration::from_secs(30 * 60));
        assert_eq!(Pace::default().quiet.as_millis() as i64, QUIET_MS);
    }

    /// Spec section 10: a card on a gateway where nothing is busy shows
    /// within 13 s, and a poll has a second for its own work. What a
    /// poll asks last may hold it for no more than is left of that
    /// after the model's slow pace, and for less than any call may take.
    #[test]
    fn what_stands_still_holds_a_poll_no_longer_than_the_idle_bound_leaves() {
        let (bound, own) = (Duration::from_secs(13), Duration::from_secs(1));
        let slow = Duration::from_millis(IDLE_POLL_MS as u64);
        let pace = Pace::default();
        assert!(slow + pace.still + own <= bound, "{:?}", pace.still);
        assert_eq!(pace.still, Duration::from_secs(2));
        assert!(pace.still < pace.answer);
    }
}
