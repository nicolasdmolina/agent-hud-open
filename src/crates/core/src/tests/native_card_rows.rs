//! A native card for a session its client's log does not name yet (ticket
//! 41): Hermes asks a question in a session it started a moment before, and
//! the HUD reads the profile's `state.db` on a later pass. The very next
//! snapshot, the one the card's wake asks for, already waits on it and tells
//! `needs-you`; the log's row then takes over without a second sound.
use super::hermes_questions::{alert_ids, asked, hermes_core, needs_you, open};
use super::*;
use agent_hud_agents::SOLE_READER;

fn rows(snapshot: &Value, id: &str) -> Vec<Value> {
    snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == id)
        .cloned()
        .collect()
}

#[test]
fn a_native_card_waits_and_sounds_at_the_next_snapshot_before_its_log_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let question = asked("question-answered-here");
    let session = question.session.hud_id();
    // The profile's log names no session yet.
    let mut core = hermes_core(temp.path(), &[]);
    core.request("snapshot", json!({})).unwrap();

    // The card's wake asks for a request pass, which reads no log.
    let _ticket = open(&core, &question);
    let snapshot = core
        .request("request_snapshot", json!({"changesOnly": true}))
        .unwrap();
    assert!(snapshot["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|card| card["id"] == question.id().as_str()));
    let row = rows(&snapshot, &session);
    assert_eq!(row.len(), 1, "one row for the carded session");
    assert_eq!(row[0]["state"], "waiting");
    assert_eq!(row[0]["source"], "hermes");
    assert_eq!(
        row[0]["liveFacts"]["requests"][0]["id"],
        question.id().as_str()
    );
    assert_eq!(needs_you(&snapshot), [json!(session)]);

    // The log names the session now: its own row takes over, still
    // waiting, and nothing sounds again.
    let db = rusqlite::Connection::open(temp.path().join(".hermes/profiles/hud-spike/state.db"))
        .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES (?1, 'tui', 'loopback', ?2, 10, 3, 'Pick a colour', 'hud-spike')",
        rusqlite::params![question.session.id(), Utc::now().timestamp() - 1],
    )
    .unwrap();
    drop(db);
    core.request("refresh", json!({})).unwrap();
    let read = core.request("snapshot", json!({})).unwrap();
    let row = rows(&read, &session);
    assert_eq!(row.len(), 1, "the log's row replaces the card's");
    assert_eq!(row[0]["evidence"], Value::Null, "the log's own row");
    assert_eq!(row[0]["state"], "waiting");
    assert_eq!(alert_ids(&read), alert_ids(&snapshot));

    // Answered elsewhere: the card goes, and so does a row only it made.
    let question = asked("question-open-ended");
    let other = question.session.hud_id();
    let _ticket = open(&core, &question);
    let shown = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(rows(&shown, &other).len(), 1);
    core.requests().settle_native(&question.id());
    core.requests()
        .set_native_waiting("hermes", SOLE_READER, &[], 0);
    let after = core.request("request_snapshot", json!({})).unwrap();
    assert!(rows(&after, &other).is_empty());
}

/// With the client's hooks on, its hooks name the session first: the row
/// only hooks made takes the card at the card's own pass, as a log row does,
/// and no second row is made for it.
#[test]
fn a_row_only_hooks_made_waits_on_its_card_at_the_next_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let question = asked("question-answered-here");
    let session = question.session.hud_id();
    let mut core = hermes_core(temp.path(), &[]);
    let now = Utc::now().timestamp_millis();
    core.ingest_hook_events(vec![agent_hud_agents::HookEvent {
        source: "hermes".into(),
        session_id: session.clone(),
        kind: "session-observation".into(),
        at_ms: now,
        data: json!({"state": "running", "turnId": "t1", "startedAtMs": now - 100, "outcome": null}),
    }]);
    let before = core.request("snapshot", json!({})).unwrap();
    let row = rows(&before, &session);
    assert_eq!(row.len(), 1);
    assert_eq!(
        row[0]["evidence"], "completion-hook",
        "a row only hooks made"
    );
    assert_eq!(row[0]["state"], "running");

    let _ticket = open(&core, &question);
    let snapshot = core
        .request("request_snapshot", json!({"changesOnly": true}))
        .unwrap();
    let row = rows(&snapshot, &session);
    assert_eq!(row.len(), 1, "the hooks' row, not a second one");
    assert_eq!(row[0]["state"], "waiting");
    assert_eq!(
        row[0]["liveFacts"]["requests"][0]["id"],
        question.id().as_str()
    );
    assert_eq!(needs_you(&snapshot), [json!(session)]);
}
