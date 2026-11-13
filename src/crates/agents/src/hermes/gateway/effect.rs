//! What goes into the gateway's state and what comes out: what happened
//! (`Input`), what to do about it (`Effect`), and why a card closed.
use super::ask::Pending;
use super::connection::{Connection, Denied, Failure};
use super::frame::Refusal;
use super::identity::SessionRef;
use super::poll::Poll;
use serde_json::Value;

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
