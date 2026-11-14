//! Why a read gave nothing, in words for the log and the machines list.
use crate::ssh_link::{self, Cut};
use agent_hud_agents::hermes::far_history::{self as far, Far, No, Rejected};

/// Why a read gave nothing. No row of it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Another reader is alive on that machine, or may be, and this one
    /// opened nothing. Worth asking again later.
    Busy,
    /// The machine has no `python3`, or a Python without `sqlite3`.
    NoHistory,
    /// The link did not run it, or was lost.
    Link(ssh_link::Failure),
    /// This side stopped waiting. The reader may still run, and the slot
    /// holds until it cannot.
    CutOff(Cut),
    /// The far program ended with this status and nothing to use.
    Exit(i32),
    /// It wrote to standard error.
    Stderr,
    Rejected(Rejected),
    /// It could not show that it is alone, did not know the request, or
    /// has a SQLite too old to bound what it reads. Nothing was opened.
    Refused(No),
    Far(Far),
    /// Even the smallest read does not fit the output bound.
    TooLarge,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("another reader is alive there"),
            Self::NoHistory => f.write_str("no history on this machine"),
            Self::Link(failure) => write!(f, "link: {failure}"),
            Self::CutOff(Cut::TimeLimit) => f.write_str("cut off at the local time limit"),
            Self::CutOff(Cut::TooMuchOutput) => f.write_str("cut off: too much output"),
            Self::CutOff(Cut::ChannelClosed) => f.write_str("cut off: the channel closed"),
            Self::Exit(status) => write!(f, "the far program ended with status {status}"),
            Self::Stderr => f.write_str("text on standard error"),
            Self::Rejected(rejected) => write!(f, "answer rejected: {rejected}"),
            Self::Refused(No::Processes) => f.write_str("the far process list could not be read"),
            Self::Refused(No::SqliteOld) => write!(
                f,
                "the far SQLite is older than {}, which cannot bound what it reads",
                far::OLDEST_SQLITE
            ),
            Self::Refused(_) => f.write_str("the far program refused the request"),
            Self::Far(far) => f.write_str(far.reason()),
            Self::TooLarge => f.write_str("too large for one read"),
        }
    }
}
