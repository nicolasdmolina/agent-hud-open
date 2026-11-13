//! What the gateway sends, read into shapes: an event, a request of its
//! own, or the reply to one of ours. Events also come back inside a replay
//! (`session.events.since`), there without the JSON-RPC envelope.
use serde_json::Value;

/// A point in one session's event stream. `epoch` names the stream; a new
/// one starts when the gateway does, and nothing replays across them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: String,
    pub seq: u64,
}

/// One gateway event. Global events name no session and carry no cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct Event<'a> {
    pub kind: &'a str,
    pub session: &'a str,
    /// The turn it belongs to, counted per session.
    pub generation: Option<u64>,
    pub at: Option<Cursor>,
    pub payload: &'a Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Frame<'a> {
    Event(Event<'a>),
    /// A request the gateway makes of its viewers (`srq-…`), answered with
    /// a result frame carrying the same id.
    Request {
        id: &'a str,
        method: &'a str,
        params: &'a Value,
    },
    /// The reply to a call, by the id the call was sent with.
    Reply {
        id: u64,
    },
    Other,
}

static NOTHING: Value = Value::Null;

/// Reads one frame from the socket.
pub fn frame(value: &Value) -> Frame<'_> {
    match (value["method"].as_str(), &value["id"]) {
        (Some("event"), _) => event(&value["params"]).map_or(Frame::Other, Frame::Event),
        (Some(method), Value::String(id)) => Frame::Request {
            id,
            method,
            params: &value["params"],
        },
        (None, id) if id.is_u64() => Frame::Reply {
            id: id.as_u64().unwrap_or_default(),
        },
        _ => Frame::Other,
    }
}

/// Reads an event's parameters, as a socket frame or a replay carries them.
pub fn event(params: &Value) -> Option<Event<'_>> {
    let at = match (params["replay_epoch"].as_str(), params["seq"].as_u64()) {
        (Some(epoch), Some(seq)) if !epoch.is_empty() => Some(Cursor {
            epoch: epoch.to_owned(),
            seq,
        }),
        _ => None,
    };
    Some(Event {
        kind: crate::text(&params["type"])?,
        session: params["session_id"].as_str().unwrap_or(""),
        generation: params["execution_generation"].as_u64(),
        at,
        payload: params.get("payload").unwrap_or(&NOTHING),
    })
}

/// Why the gateway refused a call (`error.message`, code 4001).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The turn the answer was for is over.
    StaleGeneration,
    /// This socket is not subscribed to the session.
    PermissionDenied,
    /// An answer the request does not take, such as a choice not offered.
    InvalidParams,
    /// The profile is not this socket's, or the gateway does not serve it.
    ProfileMismatch,
    /// The profile is still starting, or the gateway is restarting.
    Draining,
    /// The gateway's storage is busy (5001): worth another try.
    StorageUnavailable,
    /// A gateway that does not know the method (-32601).
    UnknownMethod,
    Other(String),
}

impl Refusal {
    /// Whether sending the same call again can work.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::PermissionDenied | Self::Draining | Self::StorageUnavailable
        )
    }
}

/// How an answer to a question or an approval was taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answered {
    Resolved,
    /// Someone else answered first, or the gateway no longer knows it.
    AlreadyResolved,
    Refused(Refusal),
}

/// The refusal in a reply frame, if it is one.
pub fn refusal(reply: &Value) -> Option<Refusal> {
    let error = reply.get("error").filter(|error| !error.is_null())?;
    let reason = error["data"]["reason"]
        .as_str()
        .or(error["message"].as_str())
        .unwrap_or("");
    Some(match reason {
        "stale_generation" => Refusal::StaleGeneration,
        "permission_denied" => Refusal::PermissionDenied,
        "invalid_params" => Refusal::InvalidParams,
        "profile_mismatch" => Refusal::ProfileMismatch,
        "runtime_draining" => Refusal::Draining,
        "storage_unavailable" => Refusal::StorageUnavailable,
        _ if error["code"] == -32601 => Refusal::UnknownMethod,
        other => Refusal::Other(other.to_owned()),
    })
}

/// Reads the reply to `clarify.respond` or `approval.respond`.
pub fn answered(reply: &Value) -> Answered {
    if let Some(refusal) = refusal(reply) {
        return Answered::Refused(refusal);
    }
    match reply["result"]["status"].as_str() {
        Some("resolved") => Answered::Resolved,
        Some("already_resolved") => Answered::AlreadyResolved,
        other => Answered::Refused(Refusal::Other(other.unwrap_or("no status").to_owned())),
    }
}
