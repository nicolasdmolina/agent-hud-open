//! A session's transcript sightings of requests (`log:` ids) beside the cards
//! waiting in it. A card stands for the sighting of the same tool call when
//! both name it (`callId`). A card whose hook names no call stands for a
//! sighting of its kind only while both are visible at once, and only when
//! that pairing is the one possible. The sighting a card stands for is hidden
//! and the card notes it (`sameAs`), so both are one request to what reads
//! them (`sound_alerts`). A sighting and a card never visible together are
//! different requests.
use serde_json::Value;

fn sighting(row: &Value) -> bool {
    row["id"].as_str().is_some_and(|id| id.starts_with("log:"))
}

/// The session's requests: its sightings, less those a card stands for,
/// then its cards.
pub(super) fn merge(sightings: Vec<Value>, pending: &[Value]) -> Vec<Value> {
    let mut cards = pending.to_vec();
    let mut hidden = vec![false; sightings.len()];
    for card in &mut cards {
        let Some(call) = card["callId"].as_str() else {
            continue;
        };
        if let Some(i) = (0..sightings.len())
            .find(|&i| !hidden[i] && sighting(&sightings[i]) && sightings[i]["callId"] == call)
        {
            hidden[i] = true;
            card["sameAs"] = sightings[i]["id"].clone();
        }
    }
    // Cards whose hook names no call hide every sighting of their kind, as
    // the HUD shows one of them; they stand for one only when it is the
    // only such sighting beside the only such card.
    let unnamed = |cards: &[Value], kind: &Value| -> Vec<usize> {
        (0..cards.len())
            .filter(|&c| cards[c]["callId"].is_null() && cards[c]["kind"] == *kind)
            .collect()
    };
    let kinds: Vec<Value> = cards.iter().map(|card| card["kind"].clone()).collect();
    for kind in kinds {
        let these = unnamed(&cards, &kind);
        if these.is_empty() {
            continue;
        }
        let open: Vec<usize> = (0..sightings.len())
            .filter(|&i| !hidden[i] && sighting(&sightings[i]) && sightings[i]["kind"] == kind)
            .collect();
        open.iter().for_each(|&i| hidden[i] = true);
        if let ([card], [i]) = (these.as_slice(), open.as_slice()) {
            cards[*card]["sameAs"] = sightings[*i]["id"].clone();
        }
    }
    let mut rows: Vec<Value> = sightings
        .into_iter()
        .zip(hidden)
        .filter_map(|(row, hidden)| (!hidden).then_some(row))
        .collect();
    rows.extend(cards);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seen(call: &str, kind: &str) -> Value {
        json!({"id":format!("log:claude:s1:{call}"),"callId":call,"sessionId":"s1","kind":kind})
    }
    fn card(id: &str, call: Option<&str>, kind: &str) -> Value {
        json!({"id":id,"callId":call,"sessionId":"s1","kind":kind})
    }
    fn ids(rows: &[Value]) -> Vec<(String, Value)> {
        rows.iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_owned(),
                    row["sameAs"].clone(),
                )
            })
            .collect()
    }

    #[test]
    fn a_card_naming_its_call_stands_for_that_sighting_only() {
        let rows = merge(
            vec![seen("a", "question"), seen("b", "question")],
            &[card("hook:1", Some("b"), "question")],
        );
        assert_eq!(
            ids(&rows),
            [
                ("log:claude:s1:a".into(), Value::Null),
                ("hook:1".into(), json!("log:claude:s1:b")),
            ]
        );
    }

    #[test]
    fn a_card_naming_no_call_stands_for_the_one_sighting_beside_it() {
        let rows = merge(
            vec![seen("a", "question")],
            &[card("hook:1", None, "question")],
        );
        assert_eq!(ids(&rows), [("hook:1".into(), json!("log:claude:s1:a"))]);
        // Another kind is another request.
        let rows = merge(vec![seen("a", "plan")], &[card("hook:1", None, "question")]);
        assert_eq!(rows.len(), 2);
        assert!(rows[1]["sameAs"].is_null());
    }

    #[test]
    fn with_several_possible_pairings_none_is_chosen() {
        let rows = merge(
            vec![seen("a", "question"), seen("b", "question")],
            &[card("hook:1", None, "question")],
        );
        assert_eq!(ids(&rows), [("hook:1".into(), Value::Null)]);
    }

    #[test]
    fn a_card_alone_names_nothing() {
        let rows = merge(vec![], &[card("hook:1", None, "permission")]);
        assert_eq!(ids(&rows), [("hook:1".into(), Value::Null)]);
    }
}
