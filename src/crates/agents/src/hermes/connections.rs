//! The Hermes desktop app's connection list, `connections.json`: text in,
//! entries out. Only the allowlisted fields are ever built (`version`, and
//! each entry's `id`, `kind`, `label`, `host`, `user`, `port`, `keyPath`).
//! No type here has a field for `token`, `headers`, `url` or anything else,
//! so their values are skipped while the text is read and never kept, and
//! the text is never read into a generic JSON value. A parse error carries
//! where and what kind, never text from the file. No I/O.
mod address;

use crate::{Location, PerOs, Support};
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;

/// The list's place: `Hermes`, the desktop app's folder, under the
/// application data folder.
pub const CONNECTIONS_FILE: Location = Location::app_data(&["Hermes"], "connections.json");
/// The list, where it is known.
pub const CONNECTIONS: PerOs<Location> = PerOs {
    macos: Support::Verified(CONNECTIONS_FILE),
    windows: Support::Unsupported(
        "Hermes's gateways are reached through unix sockets, which are not implemented on Windows",
    ),
    linux: Support::Unverified(CONNECTIONS_FILE),
};

/// The one list version this reader understands.
const VERSION: u64 = 2;

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub kind: Kind,
}

/// What an entry leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// This machine (`local`).
    This,
    /// A machine over SSH, its address split and checked.
    Ssh(SshAddress),
    /// An entry the HUD does not follow, and why.
    NotCovered(NotCovered),
}

/// An `ssh` entry's address, every part checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshAddress {
    /// A host name, an address, or an alias from the SSH configuration.
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key_path: Option<String>,
}

/// Why an entry is not followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotCovered {
    /// A gateway reached over the network.
    Remote,
    /// A Hermes Cloud gateway, whatever its sign-in.
    Cloud,
    /// A kind this reader does not know, or none.
    UnknownKind,
    /// An `ssh` entry whose host, user or port fails the checks.
    UnusableAddress,
}

/// Why the list could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListError {
    /// The text is not the list's JSON: where, and what kind of fault.
    Broken {
        line: usize,
        column: usize,
        fault: Fault,
    },
    /// The list's `version` is not one this reader understands.
    NotUnderstood,
}

/// What kind of fault broke the list, as `serde_json` classifies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Not well-formed JSON.
    Syntax,
    /// Well-formed, but a field holds the wrong type.
    Data,
    /// The text ends too early.
    Eof,
    /// Reading the text failed.
    Io,
}

impl From<serde_json::Error> for ListError {
    /// Only the position and class are kept: the error's own message can
    /// quote the file.
    fn from(error: serde_json::Error) -> Self {
        use serde_json::error::Category;
        let fault = match error.classify() {
            Category::Syntax => Fault::Syntax,
            Category::Data => Fault::Data,
            Category::Eof => Fault::Eof,
            Category::Io => Fault::Io,
        };
        Self::Broken {
            line: error.line(),
            column: error.column(),
            fault,
        }
    }
}

/// The entries of the list in `text`, in the list's order.
pub fn parse(text: &str) -> Result<Vec<Entry>, ListError> {
    // The version first, so a list of another version is "not understood"
    // whatever shape the rest of it has.
    let head: Head = serde_json::from_str(text)?;
    if head.version != Some(Scalar::Whole(VERSION)) {
        return Err(ListError::NotUnderstood);
    }
    let list: List = serde_json::from_str(text)?;
    let mut entries: Vec<Entry> = Vec::new();
    for raw in list.connections {
        let entry = raw.entry();
        // A second `local` entry is dropped, as the desktop app drops it.
        if entry.kind == Kind::This && entries.iter().any(|e| e.kind == Kind::This) {
            continue;
        }
        entries.push(entry);
    }
    Ok(entries)
}

#[derive(Deserialize)]
struct Head {
    version: Option<Scalar>,
}

#[derive(Deserialize)]
struct List {
    #[serde(default)]
    connections: Vec<RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    id: Option<String>,
    kind: Option<String>,
    label: Option<String>,
    host: Option<String>,
    user: Option<String>,
    port: Option<Scalar>,
    #[serde(rename = "keyPath")]
    key_path: Option<String>,
}

impl RawEntry {
    fn entry(self) -> Entry {
        let kind = match self.kind.as_deref() {
            Some("local") => Kind::This,
            Some("ssh") => address::address(
                self.host.as_deref().unwrap_or(""),
                self.user.as_deref(),
                self.port,
                self.key_path.as_deref(),
            )
            .map_or(Kind::NotCovered(NotCovered::UnusableAddress), Kind::Ssh),
            Some("remote") => Kind::NotCovered(NotCovered::Remote),
            Some("cloud") => Kind::NotCovered(NotCovered::Cloud),
            _ => Kind::NotCovered(NotCovered::UnknownKind),
        };
        let id = match kind {
            // The desktop app gives its `local` entry this id, always.
            Kind::This => "local".to_owned(),
            _ => trimmed(self.id),
        };
        Entry {
            id,
            label: trimmed(self.label),
            kind,
        }
    }
}

fn trimmed(text: Option<String>) -> String {
    text.as_deref().unwrap_or("").trim().to_owned()
}

/// What a `version` or `port` holds, read from any JSON value so a value
/// of another type is judged rather than breaking the list. No text is
/// kept, and nested values are skipped, never built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scalar {
    /// A whole number.
    Whole(u64),
    /// A string of digits, the whitespace around it ignored.
    Digits(u64),
    /// An empty string, or one of only whitespace.
    Empty,
    /// Anything else.
    Other,
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ScalarVisitor)
    }
}

struct ScalarVisitor;

impl<'de> Visitor<'de> for ScalarVisitor {
    type Value = Scalar;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }
    fn visit_u64<E>(self, value: u64) -> Result<Scalar, E> {
        Ok(Scalar::Whole(value))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Scalar, E> {
        Ok(u64::try_from(value).map_or(Scalar::Other, Scalar::Whole))
    }
    fn visit_f64<E>(self, value: f64) -> Result<Scalar, E> {
        // Only a whole number in `u32`'s range is taken, so the cast is exact.
        let whole = value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(if whole {
            Scalar::Whole(value as u64)
        } else {
            Scalar::Other
        })
    }
    fn visit_str<E>(self, value: &str) -> Result<Scalar, E> {
        let value = value.trim();
        Ok(if value.is_empty() {
            Scalar::Empty
        } else {
            address::digits(value).map_or(Scalar::Other, Scalar::Digits)
        })
    }
    fn visit_bool<E>(self, _: bool) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }
    fn visit_unit<E>(self) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scalar, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Scalar::Other)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scalar, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Scalar::Other)
    }
}

#[cfg(test)]
mod tests;
