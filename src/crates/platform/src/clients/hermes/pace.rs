//! How the connection paces itself: how long it waits before dialling
//! again, between polls, and for an answer. Tests shorten it.
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
}
