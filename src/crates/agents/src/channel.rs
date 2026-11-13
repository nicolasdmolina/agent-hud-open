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

/// What a native channel says of its live sessions, and when it read them:
/// the time its newest successful reading came back, which is all that
/// vouches for them. A channel that told nothing has no sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveStates {
    pub sessions: Vec<(String, LiveState)>,
    pub read_ms: i64,
}

/// The reader name a channel gives core when it alone reads its client's
/// sessions. Core keeps what a channel tells (states, waiting flags) by
/// source and reader, so a client several channels read at once names each
/// of them instead, and none replaces or outlives what another told.
pub const SOLE_READER: &str = "channel";

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

/// Where a source stands with one machine it can follow, as its native
/// channel says. A closed set: core and the UI carry the state and decide
/// nothing from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MachineState {
    Connected,
    Connecting,
    /// The machine has not been trusted yet (its host key, say).
    NotTrusted,
    CannotReach,
    NotCovered,
    /// Another of the source's entries already leads to this machine.
    SameMachine,
}

impl MachineState {
    /// The word a machine's `state` uses in the snapshot.
    pub fn word(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Connecting => "connecting",
            Self::NotTrusted => "not-trusted",
            Self::CannotReach => "cannot-reach",
            Self::NotCovered => "not-covered",
            Self::SameMachine => "same-machine",
        }
    }
}

/// One machine a source's native channel knows, as it last said: the
/// channel's id and label for it, its state, the channel's own words on it,
/// when it was last heard from and when its whole history was last checked.
/// A time the channel does not know is `None`, never zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineStatus {
    pub id: String,
    pub label: String,
    pub state: MachineState,
    pub note: Option<String>,
    pub contact_ms: Option<i64>,
    pub history_checked_ms: Option<i64>,
}

/// Everything a source's native channel says of its machines, whole: the
/// list, and apart from it, the channel's words when the list itself could
/// not be read (a note on the source, not a machine or a state of one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machines {
    pub list: Vec<MachineStatus>,
    pub list_note: Option<String>,
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
