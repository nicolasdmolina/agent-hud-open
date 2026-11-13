//! Who a frame is about: which machine, which gateway on it, which of the
//! gateway's profiles, which session. A gateway serves several profiles, a
//! machine can run several gateways and Agent HUD follows several machines,
//! so every pending item carries all of them.
use super::machine::Machine;
use crate::hermes::AGENT;
use std::fmt;
use std::path::Path;

/// One gateway Agent HUD follows: the machine it runs on and its anchor,
/// one of the profiles it serves (`anchor`). It stays the same across that
/// gateway's restarts, which its `instance_id` does not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GatewayId {
    machine: Machine,
    anchor: Profile,
}

impl GatewayId {
    pub fn new(machine: Machine, anchor: Profile) -> Self {
        Self { machine, anchor }
    }
    pub fn machine(&self) -> &Machine {
        &self.machine
    }
    pub fn anchor(&self) -> &Profile {
        &self.anchor
    }
}

/// The gateway as its request ids and its reader name write it: `local`
/// for this machine's gateway anchored at the default profile,
/// `local/<anchor>` for another of this machine's, `<key>/<anchor>` for a
/// far one. A key is hex, so it is never `local`.
impl fmt::Display for GatewayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.machine, &self.anchor) {
            (Machine::This, Profile::Default) => f.write_str("local"),
            (Machine::This, anchor) => write!(f, "local/{}", anchor.name()),
            (Machine::Far(key), anchor) => write!(f, "{}/{}", key.as_str(), anchor.name()),
        }
    }
}

/// The profile a gateway is named by, from the homes of the profiles it
/// serves, each a canonical path: the default profile if it serves it, else
/// the one whose home sorts first. The order the gateway lists them in and
/// the home that was asked do not matter. None for a gateway serving none.
pub fn anchor<'a>(served_homes: impl IntoIterator<Item = &'a str>) -> Option<Profile> {
    let mut homes: Vec<&str> = served_homes.into_iter().collect();
    homes.sort_unstable();
    let mut profiles = homes.into_iter().map(Profile::of_home);
    let first = profiles.next()?;
    let default = first == Profile::Default || profiles.any(|served| served == Profile::Default);
    Some(if default { Profile::Default } else { first })
}

/// A Hermes profile: the default one, whose home is the Hermes root, or a
/// named one under `profiles/<name>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    Default,
    Named(String),
}

impl Profile {
    /// The profile Hermes calls `name`; no name and `default` are the default.
    pub fn named(name: &str) -> Self {
        match name.trim() {
            "" | "default" => Self::Default,
            name => Self::Named(name.to_owned()),
        }
    }
    /// The profile whose home is `home`, as the gateway lists it in
    /// `served_profiles` and the hooks name it in `HERMES_HOME`.
    pub fn of_home(home: &str) -> Self {
        let home = Path::new(home);
        let inside_profiles = home
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|folder| folder == "profiles");
        match home.file_name().and_then(|name| name.to_str()) {
            Some(name) if inside_profiles => Self::named(name),
            _ => Self::Default,
        }
    }
    /// The name Hermes routes a call by (`profile: "<name>"`).
    pub fn name(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Named(name) => name,
        }
    }
}

/// A session as Agent HUD names it everywhere: the one place its id is
/// built, for the gateway's cards and states, the hooks and the `state.db`
/// reader alike. On this machine it is `hermes:<id>` in the default profile
/// and `hermes:<profile>:<id>` in a named one. On a far machine it is
/// `hermes:<key>/<profile>:<id>`, the profile always written, `default`
/// too. No profile's name holds a slash, so a far id is never a local one.
///
/// The gateway is no part of it: a profile can move from one gateway to
/// another and its sessions stay the same sessions.
pub fn hud_session_id(machine: &Machine, profile: &Profile, id: &str) -> String {
    let agent = AGENT.id;
    match (machine, profile) {
        (Machine::This, Profile::Default) => format!("{agent}:{id}"),
        (Machine::This, Profile::Named(profile)) => format!("{agent}:{profile}:{id}"),
        (Machine::Far(key), profile) => {
            format!("{agent}:{}/{}:{id}", key.as_str(), profile.name())
        }
    }
}

/// One session on one gateway.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef {
    pub gateway: GatewayId,
    pub profile: Profile,
    /// The gateway's `session_id`, which is `sessions.id` in the profile's
    /// `state.db` and `session_id` in its hook payloads.
    pub id: String,
}

impl SessionRef {
    /// The session as Agent HUD names it everywhere else (`hud_session_id`).
    pub fn hud_id(&self) -> String {
        hud_session_id(self.gateway.machine(), &self.profile, &self.id)
    }
    /// The id of one request waiting in this session, by the gateway's own
    /// id for it (a prompt id, or a server request id). The same request
    /// keeps the same id however often it is read.
    pub fn request_id(&self, asked: &str) -> String {
        format!(
            "{}:{}:{}:{}:{asked}",
            AGENT.id,
            self.gateway,
            self.profile.name(),
            self.id
        )
    }
}
