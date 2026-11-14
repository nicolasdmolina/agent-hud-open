//! A new card that names its own call, first seen beside that call's
//! transcript sighting, is one new request: it sounds once, then never again
//! for that call (ticket 47, from ticket 41's round-4 verifier probes).
use super::*;

/// One session `s` with these cards (id, kind, the call each names) and,
/// when its transcript is read, these sightings (call, kind).
fn row(
    cards: &[(&str, &str, Option<&str>)],
    sightings: &[(&str, &str)],
    transcript: bool,
) -> Value {
    let shown: Vec<_> = cards.iter().map(|(id, kind, _)| (*id, *kind)).collect();
    let mut r = waiting("s", &shown);
    if transcript {
        r["logPath"] = json!("synthetic/s.jsonl");
    }
    r["heard"] = json!(sightings
        .iter()
        .map(|(call, kind)| json!({"id":format!("log:claude:s:{call}"),"kind":kind}))
        .collect::<Vec<_>>());
    r["calls"] = json!(cards
        .iter()
        .filter_map(|(id, _, call)| call.map(|call| (id.to_string(), json!(call))))
        .collect::<serde_json::Map<_, _>>());
    r
}
fn take(tracker: &mut Tracker, r: &Value, at: i64) -> Vec<(String, String, String)> {
    look(
        tracker,
        &no_events(),
        std::slice::from_ref(r),
        &on(),
        T + at,
    )
}

#[test]
fn a_new_card_naming_its_call_beside_that_calls_sighting_sounds_once() {
    let mut tracker = started();
    let r = row(
        &[("card", "question", Some("call-1"))],
        &[("call-1", "question")],
        true,
    );
    assert_eq!(take(&mut tracker, &r, 1), [needs_you("s")]);
    assert!(take(&mut tracker, &r, 2).is_empty());
}

#[test]
fn a_new_named_card_at_a_second_takeover_sounds_once() {
    let mut tracker = started();
    let open = [
        ("named", "question", Some("a")),
        ("unnamed", "question", None),
    ];
    let both = [("a", "question"), ("b", "question")];
    assert_eq!(take(&mut tracker, &row(&open, &[], false), 1).len(), 1);
    assert!(take(&mut tracker, &row(&open, &both, true), 2).is_empty());
    assert!(take(&mut tracker, &row(&open, &[], false), 3).is_empty());
    let all = [open[0], open[1], ("new-card", "question", Some("new"))];
    let r = row(&all, &[both[0], both[1], ("new", "question")], true);
    assert_eq!(take(&mut tracker, &r, 4), [needs_you("s")]);
    assert!(take(&mut tracker, &r, 5).is_empty());
}
