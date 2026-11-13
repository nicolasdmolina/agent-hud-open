//! Who a frame is about: which gateway, which of its profiles, which
//! session. A gateway serves several profiles and Agent HUD follows several
//! gateways, so every pending item carries all three.
use crate::hermes::AGENT;
use std::path::Path;

/// One gateway Agent HUD follows, named by whoever connects to it. It stays
/// the same across that gateway's restarts, which its `instance_id` does not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GatewayId(String);

impl GatewayId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
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
    /// The session as Agent HUD names it everywhere else: `hermes:<id>` in
    /// the default profile, `hermes:<profile>:<id>` in a named one, the same
    /// as the `state.db` reader and the hooks.
    pub fn hud_id(&self) -> String {
        match &self.profile {
            Profile::Default => format!("{}:{}", AGENT.id, self.id),
            Profile::Named(profile) => format!("{}:{profile}:{}", AGENT.id, self.id),
        }
    }
    /// The id of one request waiting in this session, by the gateway's own
    /// id for it (a prompt id, or a server request id). The same request
    /// keeps the same id however often it is read.
    pub fn request_id(&self, asked: &str) -> String {
        format!(
            "{}:{}:{}:{}:{asked}",
            AGENT.id,
            self.gateway.as_str(),
            self.profile.name(),
            self.id
        )
    }
}
