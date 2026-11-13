//! The control socket's lines and the ticket it grants. The socket is asked
//! as the same user, one JSON line in and one out; the ticket opens one
//! WebSocket within 30 s and is never kept.
use super::connection::{refused, Denied, Failure, Instance};
use serde_json::{json, Value};
use std::fmt;

/// The WebSocket subprotocol the gateway speaks and must echo.
pub const SUBPROTOCOL: &str = "hermes-gateway-v1";
/// The path of the WebSocket under the instance's origin.
pub const SOCKET_PATH: &str = "/api/ws";

/// The control socket in a Hermes home, on macOS and Linux.
pub const CONTROL_SOCKET: &str = "gateway.sock";
/// Beside it where the home cannot hold a socket (a long path, a
/// filesystem that refuses one): a file naming the socket's real path.
pub const CONTROL_POINTER: &str = "gateway.sock.path";

/// The socket a pointer file names, when it is one Hermes would write: an
/// absolute `…/hermes-gw-<hash>/control.sock`. The caller still checks, as
/// Hermes's client does, that the file, the folder and the socket are the
/// user's own and closed to others.
pub fn pointed_socket(pointer: &str) -> Option<&str> {
    let path = pointer.trim();
    let folder = path.strip_suffix("/control.sock")?.rsplit('/').next()?;
    (path.starts_with('/') && path.len() <= 4096 && folder.len() > "hermes-gw-".len())
        .then_some(path)
        .filter(|_| folder.starts_with("hermes-gw-"))
}

/// What a ticket covers: the one profile it was asked for, or every profile
/// the gateway serves, each reached by naming it in a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Profile,
    Host,
}

/// The line that asks a gateway to describe itself.
pub fn identify_line() -> Value {
    json!({"protocol": 1, "id": 1, "verb": "identify"})
}

/// The line that asks `instance` for an interactive ticket. No `subject`:
/// the socket's own peer is who the ticket is for.
pub fn ticket_line(instance: &Instance, scope: Scope) -> Value {
    let mut params = json!({
        "profile_id": instance.profile_id,
        "instance_id": instance.id,
        "purpose": "interactive",
    });
    if scope == Scope::Host {
        params["scope"] = json!("host");
    }
    json!({"protocol": 1, "id": 1, "verb": "session-ticket", "params": params})
}

/// A granted ticket. It never prints: a log line or an error that holds one
/// shows that it is masked.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket(String);

impl Ticket {
    /// The subprotocols to offer when dialling; the ticket rides the second.
    pub fn subprotocols(&self) -> [String; 2] {
        [
            SUBPROTOCOL.to_owned(),
            format!("hermes-gateway-ticket.{}", self.0),
        ]
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ticket(<masked>)")
    }
}

/// The ticket in the control socket's answer, when it was granted for
/// `instance`; otherwise why not, which goes to the model as `Input::Denied`.
pub fn granted(answer: &Value, instance: &Instance) -> Result<Ticket, Denied> {
    if let Some(denied) = refused(answer) {
        return Err(denied);
    }
    let grant = match answer.get("result") {
        Some(result) if result.is_object() => result,
        _ => answer,
    };
    match crate::text(&grant["ticket"]) {
        Some(ticket) if grant["instance_id"] == instance.id.as_str() => {
            Ok(Ticket(ticket.to_owned()))
        }
        Some(_) => Err(Denied::Failed(Failure::StaleInstance)),
        None => Err(Denied::Failed(Failure::Refused(
            "no ticket in the answer".into(),
        ))),
    }
}
