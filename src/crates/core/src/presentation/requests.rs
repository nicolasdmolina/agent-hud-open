//! The requests a carded session shows: its transcript sightings (`log:`
//! ids) less every one of a kind a card waits for, then its cards.
use serde_json::Value;
use std::collections::HashMap;

/// A session's transcript requests before cards hid any, by `source:id`;
/// what the sound decision reads (`sound_alerts`), never shown.
pub(crate) type Heard = HashMap<String, Vec<Value>>;

pub(crate) fn merge(mut requests: Vec<Value>, pending: &[Value]) -> Vec<Value> {
    requests.retain(|existing| {
        !existing["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("log:"))
            || !pending.iter().any(|p| p["kind"] == existing["kind"])
    });
    requests.extend(pending.iter().cloned());
    requests
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn log(call: &str) -> Value {
        json!({"id":format!("log:claude:s1:{call}"),"sessionId":"s1","kind":"question"})
    }

    #[test]
    fn a_card_hides_every_sighting_of_its_kind_as_upstream() {
        let card = json!({"id":"hook:1","sessionId":"s1","kind":"question"});
        let plan = json!({"id":"log:claude:s1:p","sessionId":"s1","kind":"plan"});
        let rows = merge(
            vec![log("a"), log("b"), plan.clone()],
            std::slice::from_ref(&card),
        );
        assert_eq!(rows, [plan, card]);
    }
}
