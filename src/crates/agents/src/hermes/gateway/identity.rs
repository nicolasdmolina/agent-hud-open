//! Who a frame is about: which machine, which gateway on it, which of the
//! gateway's profiles, which session. A gateway serves several profiles, a
//! machine can run several gateways and Agent HUD follows several machines,
//! so every pending item carries all of them.
//!
//! Nothing here can be built from a name or an id Hermes would not write:
//! a profile's name, a session's id and the ids made of them are checked
//! where they are made, so no id of this machine can be a far machine's.
use super::machine::{canonical, Machine, StoredKey};
use crate::hermes::AGENT;
use sha2::{Digest, Sha256};
use std::fmt;
use std::num::NonZeroU16;
use std::path::Path;

/// One gateway Agent HUD follows: the machine it runs on and its anchor,
/// one of the profiles it serves (`anchor`). It stays the same across that
/// gateway's restarts, which its `instance_id` does not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GatewayId {
    machine: Machine,
    anchor: Profile,
    /// What sets it apart from another gateway of the machine that goes
    /// by the same anchor (`apart`); none for nearly every gateway.
    apart: Option<Apart>,
}

/// The first four bytes of the SHA-256 of a gateway's instance, and a
/// number for the rare two instances whose hashes begin alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Apart(u32, NonZeroU16);

impl GatewayId {
    pub fn new(machine: Machine, anchor: Profile) -> Self {
        let apart = None;
        Self {
            machine,
            anchor,
            apart,
        }
    }
    /// The id of a gateway whose anchor is already the name of another
    /// gateway that is followed: that one kept the name after the profile
    /// left it, and keeps it. This one is told apart by the first four
    /// bytes of the SHA-256 of `instance`, the run it was found as: the
    /// same for as long as it is followed.
    pub fn apart(machine: Machine, anchor: Profile, instance: &str) -> Self {
        Self::apart_again(machine, anchor, instance, NonZeroU16::MIN)
    }
    /// The same, for the `nth` gateway whose instance hashes to that
    /// start: two instances can, so whoever names a gateway takes the
    /// first `nth` whose name is free. The first is `apart`.
    pub fn apart_again(machine: Machine, anchor: Profile, instance: &str, nth: NonZeroU16) -> Self {
        let digest = Sha256::digest(instance.as_bytes());
        let start = [digest[0], digest[1], digest[2], digest[3]];
        let apart = Some(Apart(u32::from_be_bytes(start), nth));
        Self {
            machine,
            anchor,
            apart,
        }
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
/// far one. A key is hex, so it is never `local`. One told apart from a
/// gateway of the same anchor ends in `~` and eight hex characters
/// (`local/<anchor>~<8 hex>`), and in `~<n>` after that for the second
/// and later whose hashes begin alike (`local/<anchor>~<8 hex>~2`): a
/// profile's name holds no `~`, so that is never another gateway's id.
impl fmt::Display for GatewayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.machine, &self.anchor) {
            (Machine::This, Profile::Default) if self.apart.is_none() => f.write_str("local"),
            (Machine::This, anchor) => write!(f, "local/{}", anchor.name()),
            (Machine::Far(key), anchor) => write!(f, "{}/{}", key.as_str(), anchor.name()),
        }?;
        match self.apart {
            Some(Apart(hash, NonZeroU16::MIN)) => write!(f, "~{hash:08x}"),
            Some(Apart(hash, nth)) => write!(f, "~{hash:08x}~{nth}"),
            None => Ok(()),
        }
    }
}

/// The profile a gateway is named by, from the Hermes root home of its
/// machine and the homes of the profiles it serves, all canonical paths:
/// the default profile if it serves the root, else the named profile whose
/// home sorts first. The root says which home is the default profile's; a
/// home's own path cannot, since a root may itself lie in a folder called
/// `profiles`. The order the gateway lists them in and the home that was
/// asked do not matter. A served home that is no profile of this root names
/// nothing and is left out; None for a gateway serving no profile of it.
pub fn anchor<'a>(
    root_home: &str,
    served_homes: impl IntoIterator<Item = &'a str>,
) -> Option<Profile> {
    let served = served_homes.into_iter();
    // The default profile sorts before every named one, and named homes
    // under one root sort as their names do.
    served
        .filter_map(|home| Profile::in_root(root_home, home))
        .min()
}

/// A named profile's name, as Hermes allows one
/// (`hermes_constants.PROFILE_ID_RE`): 1 to 64 of `a-z`, `0-9`, `_` and
/// `-`, the first a letter or a digit. So it holds no slash and no colon,
/// and is never `default`, which is the default profile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileName(String);

impl ProfileName {
    fn new(name: &str) -> Option<Self> {
        let plain = |letter: u8| letter.is_ascii_lowercase() || letter.is_ascii_digit();
        let mut letters = name.bytes();
        let first = letters.next().is_some_and(plain);
        let rest = letters.all(|letter| plain(letter) || matches!(letter, b'_' | b'-'));
        (first && rest && name.len() <= 64 && name != "default").then(|| Self(name.to_owned()))
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
    Named(ProfileName),
}

impl Profile {
    /// The profile Hermes calls `name`; no name and `default` are the
    /// default. None for a name Hermes allows no profile, which is never
    /// tidied into one: two names are two profiles or none.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "" | "default" => Some(Self::Default),
            name => Self::folder(name),
        }
    }
    /// The named profile kept in the folder `profiles/<folder>`. None for
    /// a folder that is no profile's, `default` among them.
    pub fn folder(folder: &str) -> Option<Self> {
        ProfileName::new(folder).map(Self::Named)
    }
    /// The profile whose home is `home` in the Hermes root `root_home`,
    /// both canonical paths: the default one at the root itself, a named
    /// one at `<root>/profiles/<name>`. None for any other home.
    pub fn in_root(root_home: &str, home: &str) -> Option<Self> {
        if !canonical(root_home) {
            return None;
        }
        if home == root_home {
            return Some(Self::Default);
        }
        Self::folder(home.strip_prefix(root_home)?.strip_prefix("/profiles/")?)
    }
    /// The profile whose home is `home`, by its path alone, as the hooks
    /// (`HERMES_HOME`), the `state.db` reader and `identify` know it: the
    /// named profile `<name>` where the home is `…/profiles/<name>` and the
    /// name is one Hermes allows, else the default profile of the root that
    /// home is. Every home has a profile, so a root in any folder, of any
    /// name, is followed, read and reported. It is a guess where the root
    /// is not known: a root that is itself at `…/profiles/<name>` reads as
    /// the named profile (`in_root`, which is told the root, does not).
    pub fn of_home(home: impl AsRef<Path>) -> Self {
        let home = home.as_ref();
        let inside_profiles = home
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|folder| folder == "profiles");
        let named = home.file_name().and_then(|name| name.to_str());
        named
            .filter(|_| inside_profiles)
            .and_then(Self::folder)
            .unwrap_or(Self::Default)
    }
    /// The name Hermes routes a call by (`profile: "<name>"`).
    pub fn name(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Named(name) => name.as_str(),
        }
    }
}

/// Whether `id` can be a session's id: not empty, and no slash in it
/// before its first colon. That is where a far session's id has its key
/// (`hud_session_id`), so an id of this machine never has one there.
fn names_a_session(id: &str) -> bool {
    let first = id.split(':').next().unwrap_or_default();
    !id.is_empty() && !first.contains('/')
}

/// A session as Agent HUD names it everywhere: the one place its id is
/// built, for the gateway's cards and states, the hooks and the `state.db`
/// reader alike. On this machine it is `hermes:<id>` in the default profile
/// and `hermes:<profile>:<id>` in a named one. On a far machine it is
/// `hermes:<key>/<profile>:<id>`, the profile always written, `default`
/// too. None for an id that can be no session's (`names_a_session`).
///
/// What follows `hermes:` up to the next colon holds a slash in a far id,
/// after its key, and never in one of this machine: a profile's name holds
/// none, and an id that would put one there is refused. So a far id is
/// never a local one, whatever Hermes or a hook hands over.
///
/// The gateway is no part of it: a profile can move from one gateway to
/// another and its sessions stay the same sessions.
pub fn hud_session_id(machine: &Machine, profile: &Profile, id: &str) -> Option<String> {
    names_a_session(id).then(|| written(machine, profile, id))
}

/// A far session's id where the machine's key was read back from a file
/// Agent HUD wrote (`StoredKey`), such as a mirror: the very id
/// `hud_session_id` gives that machine's verified key, written by the same
/// hand, so a stored session and a live one are one session.
pub fn stored_session_id(key: &StoredKey, profile: &Profile, id: &str) -> Option<String> {
    names_a_session(id).then(|| write(Some(key.as_str()), profile, id))
}

fn written(machine: &Machine, profile: &Profile, id: &str) -> String {
    match machine {
        Machine::This => write(None, profile, id),
        Machine::Far(key) => write(Some(key.as_str()), profile, id),
    }
}

/// Every form of a HUD session id, by the far machine's key (None for
/// this machine).
fn write(key: Option<&str>, profile: &Profile, id: &str) -> String {
    let agent = AGENT.id;
    match (key, profile) {
        (None, Profile::Default) => format!("{agent}:{id}"),
        (None, Profile::Named(profile)) => format!("{agent}:{}:{id}", profile.as_str()),
        (Some(key), profile) => format!("{agent}:{key}/{}:{id}", profile.name()),
    }
}

/// One session on one gateway.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef {
    pub gateway: GatewayId,
    pub profile: Profile,
    /// Checked when the session is named (`new`), so every id made of it
    /// can be written.
    id: String,
}

impl SessionRef {
    /// The session the gateway calls `id` in `profile`. None for an id
    /// that can be no session's.
    pub fn new(gateway: GatewayId, profile: Profile, id: &str) -> Option<Self> {
        let id = names_a_session(id).then(|| id.to_owned())?;
        Some(Self {
            gateway,
            profile,
            id,
        })
    }
    /// The gateway's `session_id`, which is `sessions.id` in the profile's
    /// `state.db` and `session_id` in its hook payloads.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The session as Agent HUD names it everywhere else (`hud_session_id`).
    pub fn hud_id(&self) -> String {
        written(self.gateway.machine(), &self.profile, &self.id)
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
