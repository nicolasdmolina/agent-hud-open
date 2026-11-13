//! What goes into the gateway's state and what comes out: what happened
//! (`Input`), what to do about it (`Effect`), and why a card closed.
use super::ask::Pending;
use super::connection::{Connection, Denied, Failure};
use super::frame::{Cursor, Refusal};
use super::identity::{Profile, SessionRef};
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

/// One run of the gateway as the model follows it: it changes when the
/// gateway restarts. Everything read from a socket is given back with the
/// run it was asked in, and what an earlier run answers late is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run(pub(super) u64);

/// What happened. A profile comes with everything read through a call,
/// since the gateway's rows and events do not name theirs.
#[derive(Clone, Debug)]
pub enum Input<'a> {
    /// The control socket is being asked.
    Dialing,
    /// The control socket answered `identify` for the gateway under `home`.
    Identified { answer: &'a Value, home: &'a str },
    /// The control socket refused the ticket.
    Denied(Denied),
    /// The gateway could not be reached, or the socket closed.
    Lost(Failure),
    /// A frame the gateway sent on a subscribed socket as it happened: an
    /// event or a request of its own.
    Frame {
        run: Run,
        profile: &'a Profile,
        frame: &'a Value,
    },
    /// The result of `session.active_list` for one profile.
    Listed {
        run: Run,
        profile: &'a Profile,
        result: &'a Value,
    },
    /// The result of `session.events.since` for one session.
    Replayed {
        run: Run,
        profile: &'a Profile,
        session: &'a str,
        result: &'a Value,
    },
    /// The result of `session.resume`: a snapshot, and a subscription.
    Resumed {
        run: Run,
        profile: &'a Profile,
        result: &'a Value,
    },
    /// The answer to the request with this id was written to the socket.
    Sent { request: &'a str },
    /// The gateway's reply frame to an answer.
    Responded {
        run: Run,
        request: &'a str,
        reply: &'a Value,
    },
    /// Time passed: `now_ms` may be past a deadline or a recheck.
    Tick,
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
    /// Its own time ran out.
    TimedOut,
    TurnEnded,
    /// The turn was stopped.
    Interrupted,
    /// Hermes withdrew it, in its own word for why.
    Withdrawn(String),
    /// A full reading of the session no longer lists it.
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
        session: String,
        why: Closed,
    },
    /// An answer did not land and the request still waits: show its card
    /// again. `why` is the gateway's refusal, or None where no reply came.
    /// After `PermissionDenied`, subscribe to the session and answer again.
    Unanswered {
        pending: Pending,
        why: Option<Refusal>,
    },
    /// List the live sessions of every served profile now.
    List,
    /// Read the session's stream from `cursor` (`session.events.since`).
    Read {
        session: SessionRef,
        cursor: Option<Cursor>,
    },
    /// The stream cannot say what waits: take a snapshot (`session.resume`),
    /// which subscribes for as long as the socket that asks stays open.
    Resume {
        session: SessionRef,
    },
}
