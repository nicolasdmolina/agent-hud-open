//! The machine a gateway runs on: this one, or a far one. A far machine is
//! known by a key made from what its Hermes says of its own install, and
//! from nothing else: not from a connection entry, an address or a label,
//! which say where a connection was sent and never what answered it.
use sha2::{Digest, Sha256};

/// One Hermes install in one home.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Machine {
    /// The machine Agent HUD runs on.
    This,
    Far(MachineKey),
}

/// A far machine's key, 16 lowercase hex characters. `machine_key` is the
/// only way to make one, so every key comes from a verified identity. The
/// install id it was made from cannot be read back out of it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineKey(String);

impl MachineKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an identity makes no key. None of them holds what was reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoKey {
    /// Hermes there named no install id at all.
    Missing,
    /// It named an empty one, which Hermes does when it can neither read
    /// nor write its id (`gateway/session_group_controls.py`).
    Empty,
    /// Not 32 characters.
    Length,
    /// Hex, but not all lower case, which Hermes never writes.
    UpperCase,
    /// A character that is no hex digit.
    NotHex,
    /// The root home is not a whole path from the root, so it was not made
    /// canonical on the far side.
    Home,
}

impl NoKey {
    /// A word for the log.
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::Missing => "install_id_missing",
            Self::Empty => "install_id_empty",
            Self::Length => "install_id_length",
            Self::UpperCase => "install_id_upper_case",
            Self::NotHex => "install_id_not_hex",
            Self::Home => "root_home_not_canonical",
        }
    }
}

/// How many characters Hermes writes for an install id: a UUID as hex
/// (`hermes_cli/install_identity.py`).
const INSTALL_ID_LENGTH: usize = 32;
const KEY_LENGTH: usize = 16;

/// The key of the far machine whose Hermes reports `install_id` and whose
/// root home is `root_home`. The id is read over a socket Agent HUD
/// authenticated, and the home is made canonical on the far side: this
/// only checks their shape. An id that is not exactly 32 lowercase hex
/// characters makes no key, and nothing stands in for it.
///
/// The key is the first 16 hex characters of the SHA-256 of the id, a line
/// feed and the home. One install copied to two homes is two machines.
pub fn machine_key(install_id: Option<&str>, root_home: &str) -> Result<MachineKey, NoKey> {
    let id = install_id.ok_or(NoKey::Missing)?;
    if id.is_empty() {
        return Err(NoKey::Empty);
    }
    if id.chars().count() != INSTALL_ID_LENGTH {
        return Err(NoKey::Length);
    }
    if !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(NoKey::NotHex);
    }
    if id.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(NoKey::UpperCase);
    }
    if !root_home.starts_with('/') {
        return Err(NoKey::Home);
    }
    let mut hash = Sha256::new();
    hash.update(id.as_bytes());
    hash.update(b"\n");
    hash.update(root_home.as_bytes());
    let hex: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(MachineKey(hex[..KEY_LENGTH].to_owned()))
}

/// Whether the gateways found on one route all report the same install
/// id, each given as it reported it. A gateway that names none counts as
/// naming an empty one. A route whose gateways disagree leads to no one
/// machine, and is not covered until they agree.
pub fn identities_agree<'a>(install_ids: impl IntoIterator<Item = Option<&'a str>>) -> bool {
    let mut reported = install_ids.into_iter().map(Option::unwrap_or_default);
    match reported.next() {
        Some(first) => reported.all(|id| id == first),
        None => true,
    }
}
