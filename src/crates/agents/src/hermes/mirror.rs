//! A mirror: a local file in the shape of Hermes's `state.db` that holds
//! one far profile's sessions and usage. Its `hud_mirror` table says so,
//! never its path, and says whose: the far machine's key, the label its
//! sessions show and the far profile. Agent HUD's mirror writer is the only
//! one that writes it.
use super::gateway::{Profile, StoredKey};
use crate::transcript::sql_columns;
use rusqlite::{types::Value as Sql, Connection};

/// The one `hud_mirror.schema` this reader knows.
const SCHEMA: i64 = 1;

/// Whose sessions a mirror holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mirror {
    /// The far machine's key as the writer stored it. It names the
    /// machine's sessions and proves nothing.
    pub key: StoredKey,
    /// The label its sessions show (`Record::machine`).
    pub label: String,
    /// The far profile, from the table and never from the file's folder.
    pub profile: Profile,
}

/// The mirror `db` is, None for a database without `hud_mirror`, which is
/// Hermes's own and reads as it always has. A mirror whose row cannot be
/// trusted is refused whole, so nothing of it is read: an unknown
/// `schema`, a `machine` that is not a stored key, an empty `label`, a
/// `profile` Hermes allows no profile to be named, or not exactly one row.
pub(super) fn identity(db: &Connection) -> rusqlite::Result<Option<Mirror>> {
    if sql_columns(db, "hud_mirror")?.is_empty() {
        return Ok(None);
    }
    let mut statement =
        db.prepare("SELECT schema, machine, label, profile FROM hud_mirror LIMIT 2")?;
    let rows = statement
        .query_map([], |row| {
            Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?])
        })?
        .collect::<rusqlite::Result<Vec<[Sql; 4]>>>()?;
    let mirror = match rows.as_slice() {
        [row] => trusted(row),
        _ => None,
    };
    mirror.map(Some).ok_or(rusqlite::Error::InvalidQuery)
}

/// The mirror one `hud_mirror` row describes, None where any part of it
/// is not as the writer writes it. Nothing is tidied.
fn trusted([schema, machine, label, profile]: &[Sql; 4]) -> Option<Mirror> {
    if *schema != Sql::Integer(SCHEMA) {
        return None;
    }
    Some(Mirror {
        key: StoredKey::parse(text(machine)?)?,
        label: text(label).filter(|label| !label.is_empty())?.to_owned(),
        profile: match text(profile)? {
            "default" => Profile::Default,
            name => Profile::folder(name)?,
        },
    })
}

fn text(value: &Sql) -> Option<&str> {
    match value {
        Sql::Text(text) => Some(text),
        _ => None,
    }
}
