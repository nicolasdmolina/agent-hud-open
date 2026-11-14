//! A gateway on a far machine: what is asked of its control socket, and
//! what the relay that asks it there says back. This is the pure half.
//! Platform sends the relay over its SSH link and runs it
//! (`platform/src/clients/hermes/far.rs`).
//!
//! Only two things are ever asked: `identify` and `session-ticket`
//! ([`Request`]). The same socket takes verbs that pause a gateway, stop
//! serving a profile, and rename or purge one; no request for those can be
//! made here, and the relay refuses a line that names one.
use super::ticket::Scope;
use serde_json::{json, Value};
use std::fmt;

mod homes;
mod picture;

pub use homes::{homes, FarHome, Homes, NoHomes, HOMES_SCRIPT, HOMES_SECONDS};
pub use picture::{picture, Asked, FoundGateway, Picture};

/// How long the relay lives on the far machine, in seconds, to begin with.
pub const RELAY_SECONDS: u32 = 10;
/// The longest answer the relay passes on; Hermes's own client stops at
/// the same size.
pub const MOST_ANSWER: usize = 512 * 1024;
/// How the relay ends at its deadline. It says nothing then.
const AT_DEADLINE: i32 = 3;
/// How `sh` ends when there is no `python3` to run, or it cannot be run.
const NOT_FOUND: [i32; 2] = [126, 127];

/// One request to a far control socket. These two are all there is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Identify,
    /// An interactive ticket from the instance `instance_id`, for the
    /// profile `profile_id` or, with [`Scope::Host`], every one it serves.
    Ticket {
        profile_id: &'a str,
        instance_id: &'a str,
        scope: Scope,
    },
}

impl<'a> Request<'a> {
    /// The request a control line is, when it is one of the two exactly
    /// as [`Request::line`] writes them. A route is handed a line
    /// (`Route::exchange`); this is how it gets the request to send, and
    /// a line that asks anything else gives none.
    pub fn of(line: &'a Value) -> Option<Self> {
        let request = match line["verb"].as_str()? {
            "identify" => Self::Identify,
            "session-ticket" => Self::Ticket {
                profile_id: line["params"]["profile_id"].as_str()?,
                instance_id: line["params"]["instance_id"].as_str()?,
                scope: match line["params"].get("scope") {
                    None => Scope::Profile,
                    Some(_) => Scope::Host,
                },
            },
            _ => return None,
        };
        // Nothing more in it, and nothing written another way.
        (serde_json::from_str::<Value>(&request.line()).ok().as_ref() == Some(line))
            .then_some(request)
    }

    /// The line sent, as Hermes's own client writes it
    /// (`ticket::identify_line`, `ticket::ticket_line`).
    pub fn line(&self) -> String {
        match *self {
            Self::Identify => json!({"protocol": 1, "id": 1, "verb": "identify"}),
            Self::Ticket {
                profile_id,
                instance_id,
                scope,
            } => {
                let mut params = json!({
                    "profile_id": profile_id,
                    "instance_id": instance_id,
                    "purpose": "interactive",
                });
                if scope == Scope::Host {
                    params["scope"] = json!("host");
                }
                json!({"protocol": 1, "id": 1, "verb": "session-ticket", "params": params})
            }
        }
        .to_string()
    }
}

/// What a far control socket answered. It can hold a ticket, so it never
/// prints: a log line or a failed assertion shows that it is masked.
#[derive(Clone, PartialEq, Eq)]
pub struct Answered(Value);

impl Answered {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

impl fmt::Debug for Answered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Answered(<masked>)")
    }
}

/// Why the relay brought no answer back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quiet {
    /// No home, no socket in it, or nobody listening: no gateway runs.
    Absent,
    /// The gateway took the request and closed without a word. Worth one
    /// more try (measured once on a real machine, right after `identify`).
    Empty,
    /// The relay ended at its deadline: the gateway did not answer.
    TimedOut,
    /// The answer was longer than [`MOST_ANSWER`].
    TooLong,
    /// What came back is not one JSON line.
    Garbled,
    /// The relay itself failed, or said something that is no status of its.
    Failed,
    /// A check failed, or could not be made. Nothing was sent.
    Unchecked(Unchecked),
}

/// A check the relay makes before it sends anything, which failed or could
/// not be made. Each is the machine's reason for not being covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unchecked {
    /// No `python3` there to run the relay.
    NoPython,
    /// The home is a link, or is not written the one way its folder is.
    HomeNotCanonical,
    HomeNotYours,
    /// Someone other than the login user may write the home.
    HomeOpenToOthers,
    SocketNotYours,
    SocketOpenToOthers,
    PointerIsALink,
    PointerNotYours,
    PointerOpenToOthers,
    PointerTooLong,
    /// The pointer names something other than the socket Hermes makes for
    /// this home.
    PointerOutside,
    FolderNotYours,
    FolderOpenToOthers,
    /// That system cannot say who listens on a socket.
    NoPeerCredentials,
    /// The process that listens runs as another user.
    ListenerNotYours,
    /// The line named a verb that is never sent.
    VerbNotSent,
}

const UNCHECKED: [(Unchecked, &str); 16] = [
    (Unchecked::NoPython, "no-python3"),
    (Unchecked::HomeNotCanonical, "home-not-canonical"),
    (Unchecked::HomeNotYours, "home-not-yours"),
    (Unchecked::HomeOpenToOthers, "home-open-to-others"),
    (Unchecked::SocketNotYours, "socket-not-yours"),
    (Unchecked::SocketOpenToOthers, "socket-open-to-others"),
    (Unchecked::PointerIsALink, "pointer-is-a-link"),
    (Unchecked::PointerNotYours, "pointer-not-yours"),
    (Unchecked::PointerOpenToOthers, "pointer-open-to-others"),
    (Unchecked::PointerTooLong, "pointer-too-long"),
    (Unchecked::PointerOutside, "pointer-outside"),
    (Unchecked::FolderNotYours, "folder-not-yours"),
    (Unchecked::FolderOpenToOthers, "folder-open-to-others"),
    (Unchecked::NoPeerCredentials, "no-peer-credentials"),
    (Unchecked::ListenerNotYours, "listener-not-yours"),
    (Unchecked::VerbNotSent, "verb-not-sent"),
];

impl Unchecked {
    /// The reason's name: the word the relay says, and a word for a log.
    pub fn reason_code(self) -> &'static str {
        UNCHECKED
            .iter()
            .find(|(reason, _)| *reason == self)
            .map_or("unchecked", |(_, code)| code)
    }
}

/// What the relay said, from how it ended and its standard output: a
/// status line, and after `ok` the one line the gateway answered. Nothing
/// of the output is kept in a refusal, and standard error is never read:
/// either could hold a ticket.
pub fn relay_said(exit: i32, output: &[u8]) -> Result<Answered, Quiet> {
    if NOT_FOUND.contains(&exit) {
        return Err(Quiet::Unchecked(Unchecked::NoPython));
    }
    if exit == AT_DEADLINE {
        return Err(Quiet::TimedOut);
    }
    let mut lines = output.splitn(2, |byte| *byte == b'\n');
    let status = std::str::from_utf8(lines.next().unwrap_or_default()).unwrap_or_default();
    let rest = lines.next().unwrap_or_default();
    if status != "ok" {
        return Err(match status {
            "no-home" | "no-socket" | "nobody-listening" => Quiet::Absent,
            "empty-answer" => Quiet::Empty,
            "answer-too-long" => Quiet::TooLong,
            "answer-cut-short" => Quiet::Garbled,
            named => UNCHECKED
                .iter()
                .find(|(_, code)| *code == named)
                .map_or(Quiet::Failed, |(reason, _)| Quiet::Unchecked(*reason)),
        });
    }
    if exit != 0 {
        return Err(Quiet::Failed);
    }
    if rest.len() > MOST_ANSWER {
        return Err(Quiet::TooLong);
    }
    // One line and its end, and nothing after it.
    let line = rest.strip_suffix(b"\n").ok_or(Quiet::Garbled)?;
    if line.contains(&b'\n') {
        return Err(Quiet::Garbled);
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(answer) if answer.is_object() => Ok(Answered(answer)),
        _ => Err(Quiet::Garbled),
    }
}
