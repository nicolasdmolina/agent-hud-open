//! Native channels: client mechanisms that need the operating system, such
//! as a desktop app's IPC or a local server the client runs. Platform runs
//! each in `platform/src/clients/<id>/`; the descriptor says what it carries
//! and where it runs.
use crate::{Os, PerOs};

/// What a native channel carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carries {
    /// Requests the client waits on (approvals, questions, plans) and the
    /// answers to them.
    Requests,
    /// The account's quota.
    Quota,
}

/// What a native channel says a live session is doing, where it can say:
/// a turn under way, a turn that waits on the person, or no turn. Core lets
/// it stand over what the hooks and the log say for the sessions it names
/// (`agent_hud_core::requests::Requests::set_native_states`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiveState {
    Running,
    Waiting,
    Idle,
}

impl LiveState {
    /// The word a session row's `state` uses.
    pub fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
        }
    }
}

pub struct Channel {
    /// What it is, in a few words.
    pub name: &'static str,
    pub carries: &'static [Carries],
    /// Where it runs. An unsupported OS never starts it.
    pub support: PerOs<()>,
}

impl Channel {
    /// Whether it runs on `os`.
    pub fn runs_on(&self, os: Os) -> bool {
        self.support.on(os).value().is_some()
    }
}
