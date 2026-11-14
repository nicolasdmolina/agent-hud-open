//! What goes into the gateway's state and what comes out: what happened
//! (`Input`), what to do about it (`Effect`), and why a card closed.
use super::ask::Pending;
use super::connection::{Connection, Denied, Failure};
use super::frame::Refusal;
use super::identity::{Profile, SessionRef};
use super::poll::Poll;
use crate::LiveState;
use serde_json::Value;
use std::collections::BTreeSet;

/// What a session is doing, as `session.active_list` says it. A session
/// waiting on a set of questions reports `working`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Working,
    Waiting,
    Idle,
}

impl Status {
    /// What the HUD shows for it. `starting` is a turn Hermes took in whose
    /// agent has not begun (`gateway/session_active_list.py`, `_status`):
    /// work under way.
    pub fn live(self) -> LiveState {
        match self {
            Self::Starting | Self::Working => LiveState::Running,
            Self::Waiting => LiveState::Waiting,
            Self::Idle => LiveState::Idle,
        }
    }
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "starting" => Self::Starting,
            "working" => Self::Working,
            "waiting" => Self::Waiting,
            "idle" => Self::Idle,
            _ => return None,
        })
    }
}

/// What happened.
#[derive(Debug)]
pub enum Input<'a> {
    /// The control socket is being asked.
    Dialing,
    /// The control socket answered `identify` for the gateway under `home`.
    Identified { answer: &'a Value, home: &'a str },
    /// The gateway this model followed is gone: its home answers as another
    /// instance. Nothing it asked survives; whoever follows the new
    /// instance starts from nothing (`served.rs`).
    Restarted,
    /// These profiles are no longer this gateway's: its own verified answer
    /// dropped them, or a verified discovery shows another instance serving
    /// them. Their cards close and their sessions are forgotten.
    Retired(&'a BTreeSet<Profile>),
    /// The profiles more than one instance claims just now. No card is
    /// opened for a session of one of them until this says otherwise; the
    /// cards that show are kept, and close as Hermes says.
    Withheld(&'a BTreeSet<Profile>),
    /// The control socket refused the ticket.
    Denied(Denied),
    /// The gateway could not be reached, or the socket closed.
    Lost(Failure),
    /// A poll ran to its end: every step it asked for was answered or
    /// failed. A poll that was cut off is dropped, not given.
    Polled(Poll),
    /// The answer to the request with this id was written to the socket.
    Sent { request: &'a str },
    /// The gateway's reply frame to an answer.
    Responded { request: &'a str, reply: &'a Value },
    /// An answer to the request with this id was taken and could not be
    /// sent: nothing of it was written. Its gateway is in doubt until a
    /// poll finds the request waiting again (`reach.rs`).
    Unsent { request: &'a str },
}

/// Why a card closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Closed {
    /// Answered here, and Hermes took the answer.
    Answered,
    /// Hermes says it settled: answered elsewhere, or Hermes stopped waiting.
    Settled,
    AlreadyResolved,
    StaleGeneration,
    /// Hermes withdrew a set of questions when its own wait ran out.
    TimedOut,
    TurnEnded,
    /// The turn was stopped.
    Interrupted,
    /// Hermes withdrew it, in its own word for why.
    Withdrawn(String),
    /// What Hermes says of the session no longer holds it.
    Gone,
    SessionEnded,
    /// The session's profile left this gateway while it runs: it serves
    /// the profile no longer, or another instance does.
    Retired,
    GatewayRestarted,
    Unsupported,
}

/// What to do. The connection carries each out and nothing else decides.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Connection(Connection),
    /// Show the card: register the request.
    Open(Pending),
    /// The card changed while it waits: show it as it is now, under the
    /// request it already has (another viewer answered one of its questions).
    Changed(Pending),
    /// The card's gateway cannot be reached, and Hermes may still wait:
    /// the card stays, changed in place to say so, and takes no answer
    /// (`reach.rs`). Never a second request.
    Marked(Pending),
    /// A poll found the request waiting again: the card is as it was.
    Unmarked(Pending),
    /// Withdraw the card: the request is over.
    Close {
        id: String,
        session: SessionRef,
        why: Closed,
    },
    /// An answer did not land and the request still waits: show its card
    /// again. `why` is the gateway's refusal, or None where no reply came.
    /// After `PermissionDenied`, subscribe to the session and answer again.
    Unanswered {
        pending: Pending,
        why: Option<Refusal>,
    },
    /// Poll now, without waiting for the timer.
    Poll,
}
