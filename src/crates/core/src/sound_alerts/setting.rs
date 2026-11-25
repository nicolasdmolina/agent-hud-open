//! What a sound setting may hold.
use serde_json::Value;

/// Whether a setting names a sound: a name, never a path. The host looks it
/// up among the system's own sounds.
pub(crate) fn names_a_sound(value: &Value) -> bool {
    value.as_str().is_some_and(|name| {
        !name.is_empty()
            && name.trim() == name
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || " -_".contains(c))
    })
}
