//! The calls Agent HUD makes on the gateway's WebSocket, as the exact
//! frames. Reading (`session.active_list`, `session.events.since`) needs no
//! subscription; `session.resume` subscribes, which answering requires.
use super::frame::Cursor;
use super::identity::{Profile, SessionRef};
use super::ticket::Scope;
use serde_json::{json, Value};

/// One call; the connection numbers it when it sends it.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub method: &'static str,
    pub params: Value,
}

impl Command {
    /// The JSON-RPC frame, with the integer id its reply will carry.
    pub fn frame(&self, id: u64) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": self.method, "params": self.params})
    }
    /// The call for `profile` on a socket of `scope`: a socket for the whole
    /// host names the profile in every call, a profile's own never does.
    pub(super) fn routed(
        method: &'static str,
        mut params: Value,
        profile: &Profile,
        scope: Scope,
    ) -> Self {
        if scope == Scope::Host {
            params["profile"] = json!(profile.name());
        }
        Self { method, params }
    }
}

/// Every live session of one profile with its status. A host's socket
/// lists one profile per call, never all of them at once.
pub fn active_list(profile: &Profile, scope: Scope) -> Command {
    Command::routed("session.active_list", json!({}), profile, scope)
}

/// The profiles of the install, for the one thing a far gateway is asked
/// on its first socket: the install's own id (`machine::reported_install`).
/// Without the sessions of each profile, which nothing here reads.
pub fn profiles_list(profile: &Profile, scope: Scope) -> Command {
    let params = json!({"include_sessions": false});
    Command::routed("profiles.list", params, profile, scope)
}

/// What a session's stream holds after `cursor`. With none, the answer
/// only tells the stream's epoch; the next read then starts at 0 in that
/// epoch, which returns what is still held, a waiting request included.
pub fn events_since(session: &SessionRef, cursor: Option<&Cursor>, scope: Scope) -> Command {
    let mut params = json!({
        "session_id": session.id(),
        "last_sequence": cursor.map_or(0, |cursor| cursor.seq),
    });
    if let Some(cursor) = cursor {
        params["replay_epoch"] = json!(cursor.epoch);
    }
    Command::routed("session.events.since", params, &session.profile, scope)
}

/// Subscribes to a session and returns its snapshot: every prompt and
/// request waiting now. It is the only way to answer, and the only full
/// reading once a replay is cut short.
pub fn resume(session: &SessionRef, scope: Scope) -> Command {
    Command::routed(
        "session.resume",
        json!({"session_id": session.id()}),
        &session.profile,
        scope,
    )
}

/// Ends a subscription. No `end_reason`: that would close the session
/// itself, which is never Agent HUD's to do.
pub fn detach(session: &SessionRef, subscription: &str, scope: Scope) -> Command {
    Command::routed(
        "session.detach",
        json!({"session_id": session.id(), "subscription_id": subscription}),
        &session.profile,
        scope,
    )
}
