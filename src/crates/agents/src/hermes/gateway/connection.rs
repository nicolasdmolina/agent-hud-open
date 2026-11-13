//! The state of the connection to one gateway, and what the gateway's own
//! description of itself (the control socket's `identify` answer) makes it.
//! The rules are the ones Hermes's own client applies
//! (`hermes_cli/gateway_runtime.py`, `_endpoint`): the contract's version
//! string does not change between incompatible builds, so a build is told by
//! `runtime_protocol` and its capabilities.
use super::identity::Profile;
use serde_json::Value;
use url::{Host, Url};

/// The capability a gateway needs before another client may attach to the
/// sessions on it.
pub const SESSION_AUTHORITY: &str = "session-authority-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Disconnected,
    Connecting,
    Ready(Instance),
    /// A gateway without the live connection: the hooks alone report it.
    Unsupported(Unsupported),
    Failed(Failure),
}

/// One run of a gateway. A restart is a new instance: nothing asked before
/// it survives, and nothing can be replayed across it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub id: String,
    pub authority_epoch: u64,
    /// `http://127.0.0.1:<port>`, checked to be this machine. A restart
    /// usually moves the port but need not.
    pub origin: String,
    pub port: u16,
    /// `profile_id` of the home whose socket was asked, for the ticket.
    pub profile_id: String,
    /// Every profile this gateway serves; a list call covers one of them.
    pub served: Vec<Served>,
    /// The gateway waits for its turns to end before a restart: what waits
    /// can still be answered, which lets the restart finish.
    pub draining: bool,
    /// Informative only; never decides support.
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Served {
    pub profile: Profile,
    pub profile_id: String,
}

/// Why a gateway lacks the live connection, by Hermes's own reason codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    RuntimeProtocol,
    SessionAuthority,
    /// Its control socket does not know the `session-ticket` verb.
    TicketVerb,
    /// Its API is not plain HTTP on this machine, the one kind Agent HUD
    /// dials: the local gateway serves it, and a far gateway is reached
    /// through a forwarded port of this machine that serves it too.
    Origin,
}

impl Unsupported {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::RuntimeProtocol => "runtime_protocol",
            Self::SessionAuthority => "session_authority_unavailable",
            Self::TicketVerb => "unknown_verb",
            Self::Origin => "api_origin_not_local_http",
        }
    }
}

/// Why a supported gateway could not be reached or used. All of them are
/// worth another try: identify again, ask for a new ticket, dial again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No control socket, or nothing listening on it: the gateway is stopped.
    Unreachable,
    /// The gateway does not serve the profile whose socket was asked.
    ProfileNotServed,
    /// What identify answered cannot be used, by Hermes's reason code.
    Invalid(&'static str),
    /// The instance named in a ticket request has been replaced.
    StaleInstance,
    /// The WebSocket upgrade was refused: a ticket is single use and lasts 30 s.
    TicketRefused,
    /// The socket closed, with its close code (1012 restart, 1006 killed).
    Closed(u16),
    /// Any other refusal, in the gateway's words.
    Refused(String),
}

/// What an `identify` answer makes the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identified {
    Ready(Instance),
    /// Starting, draining or in conflict: ask again, never fall back.
    Wait(String),
    Unsupported(Unsupported),
    Failed(Failure),
}

/// Reads the control socket's answer to `identify` for the gateway whose
/// socket is under `home`. `home` is compared as written: the caller resolves
/// links before asking, as Hermes's client does.
pub fn identify(answer: &Value, home: &str) -> Identified {
    if let Some(denied) = refused(answer) {
        return denied.into();
    }
    let told = match answer.get("result") {
        Some(result) if result.is_object() => result,
        _ => answer,
    };
    if told["runtime_protocol"].as_u64() != Some(1) {
        return Identified::Unsupported(Unsupported::RuntimeProtocol);
    }
    // A gateway that is not ready yet may not list its profiles yet either,
    // so its state is read first: wait, whatever else it says.
    let state = told["state"].as_str().unwrap_or("");
    let draining = state == "draining" && told["drain_reason"] == "restart_wait";
    if matches!(state, "starting" | "draining" | "conflict") && !draining {
        return Identified::Wait(state.to_owned());
    }
    let served: Vec<Served> = told["served_profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let profile_id = crate::text(&entry["profile_id"])?;
            Some((crate::text(&entry["home"])?, profile_id))
        })
        .map(|(home, profile_id)| Served {
            profile: Profile::of_home(home),
            profile_id: profile_id.to_owned(),
        })
        .collect();
    let same =
        |a: &str, b: &str| a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\']);
    let mut homes = told["served_profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry["home"]
                .as_str()
                .is_some_and(|served| same(served, home))
        });
    let profile_id = match (homes.next(), homes.next()) {
        (Some(entry), None) => crate::text(&entry["profile_id"]),
        _ => None,
    };
    let Some(profile_id) = profile_id else {
        return Identified::Failed(Failure::ProfileNotServed);
    };
    if state != "ready" && !draining {
        return Identified::Failed(Failure::Invalid("invalid_runtime_state"));
    }
    let capable = told["capabilities"].as_array().is_some_and(|all| {
        all.iter().all(Value::is_string) && all.iter().any(|c| c == SESSION_AUTHORITY)
    });
    if !capable {
        return Identified::Unsupported(Unsupported::SessionAuthority);
    }
    let (Some(authority_epoch), Some(id)) = (
        told["authority_epoch"].as_u64().filter(|epoch| *epoch > 0),
        crate::text(&told["instance_id"]),
    ) else {
        return Identified::Failed(Failure::Invalid("invalid_runtime_identity"));
    };
    // Never dialled: it is left to the hooks, like a build without the
    // live connection.
    let Some((origin, port)) = told["api_origin"].as_str().and_then(local_origin) else {
        return Identified::Unsupported(Unsupported::Origin);
    };
    Identified::Ready(Instance {
        id: id.to_owned(),
        authority_epoch,
        origin,
        port,
        profile_id: profile_id.to_owned(),
        served,
        draining,
        version: crate::text(&told["code_version"]).map(str::to_owned),
    })
}

/// A control socket refusal (`{ok: false, error}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The socket does not know the verb: a build without the connection.
    Unsupported(Unsupported),
    Failed(Failure),
}

impl From<Denied> for Identified {
    fn from(denied: Denied) -> Self {
        match denied {
            Denied::Unsupported(why) => Self::Unsupported(why),
            Denied::Failed(failure) => Self::Failed(failure),
        }
    }
}

/// The refusal in a control socket's answer; None for any other answer.
pub fn refused(answer: &Value) -> Option<Denied> {
    if answer["ok"] != false {
        return None;
    }
    let error = answer["error"].as_str().unwrap_or("").trim();
    Some(if error.starts_with("unknown verb") {
        Denied::Unsupported(Unsupported::TicketVerb)
    } else if error.contains("stale runtime instance") {
        Denied::Failed(Failure::StaleInstance)
    } else if error.contains("bootstrap binding rejected") {
        Denied::Failed(Failure::ProfileNotServed)
    } else {
        Denied::Failed(Failure::Refused(error.to_owned()))
    })
}

/// The origin and its port, when it is plain `http://<loopback>:<port>`.
/// No TLS: nothing but this machine is ever dialled.
fn local_origin(origin: &str) -> Option<(String, u16)> {
    let url = Url::parse(origin).ok()?;
    let local = match url.host()? {
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
        Host::Domain(_) => false,
    };
    let plain = url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none();
    (local && plain).then_some((origin.trim_end_matches('/').to_owned(), url.port()?))
}
