//! Hermes's `state.db`, built from the real schema with synthetic rows:
//! running totals that grow while a session works.
use super::*;
use crate::{
    checkpoints::{self, LogHead},
    SourceRoot,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::fs;

const SCHEMA: &str = include_str!("../../../agents/tests/fixtures/hermes/macos/state.schema.sql");
const WITHOUT_LAST_ACTIVITY: &str = include_str!(
    "../../../agents/tests/fixtures/hermes/macos/state-without-last-activity.schema.sql"
);
const PERIOD_MS: i64 = 15 * 60 * 1000;

fn hermes(root: &Path, schema: &str) -> (Connection, AgentHudCore) {
    let folder = root.join("hermes");
    fs::create_dir_all(&folder).unwrap();
    let db = Connection::open(folder.join("state.db")).unwrap();
    db.execute_batch(schema).unwrap();
    (db, backend(root))
}

fn backend(root: &Path) -> AgentHudCore {
    AgentHudCore::new(
        vec![SourceRoot {
            path: root.join("hermes").join("state.db"),
            provider: "hermes",
        }],
        root.join("hud"),
    )
}

fn refresh(core: &mut AgentHudCore) -> Value {
    let mut snapshot = core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        if core.reads.pending.is_empty() {
            return snapshot;
        }
        snapshot = core.request("snapshot", json!({})).unwrap();
    }
    panic!("the Hermes fixture did not finish indexing");
}

/// A time three hours ago at the start of a 15-minute period, in ms.
fn start() -> i64 {
    (chrono::Utc::now().timestamp_millis() / PERIOD_MS - 12) * PERIOD_MS
}

fn seconds(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

fn session(db: &Connection, started: i64, input: i64, output: i64) {
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at) VALUES('s1','cli','claude-sonnet-4-5',?1)",
        [seconds(started)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage(session_id,model,input_tokens,output_tokens,first_seen,last_seen)
         VALUES('s1','claude-sonnet-4-5',?1,?2,?3,?3)",
        params![input, output, seconds(started)],
    )
    .unwrap();
}

/// Hermes adds to the usage row in place and stamps when it last grew.
fn grow(db: &Connection, input: i64, output: i64, at: i64) {
    db.execute(
        "UPDATE session_model_usage SET input_tokens=?1,output_tokens=?2,last_seen=?3",
        params![input, output, seconds(at)],
    )
    .unwrap();
}

/// Each usage event in the ledger: (time, input, output).
fn events(core: &AgentHudCore) -> Vec<(i64, i64, i64)> {
    let db = core.database.as_ref().unwrap();
    let mut statement = db
        .prepare("SELECT at_ms,input,output FROM usage_meter_event ORDER BY at_ms")
        .unwrap();
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn tokens(core: &mut AgentHudCore) -> Value {
    core.request("session_detail", json!({"id":"hermes:s1"}))
        .unwrap()["usage"]["total"]
        .clone()
}

#[test]
fn a_running_hermes_sessions_growth_is_dated_at_each_observation() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), SCHEMA);
    let first = start();
    session(&db, first, 100, 10);
    refresh(&mut core);
    assert_eq!(events(&core), [(first, 100, 10)]);

    // An hour later the session is still running and has grown.
    let seen = first + 3_600_000 + 1_000;
    grow(&db, 300, 30, seen);
    db.execute("UPDATE sessions SET last_activity_at=?1", [seconds(seen)])
        .unwrap();
    refresh(&mut core);
    assert_eq!(events(&core), [(first, 100, 10), (seen, 200, 20)]);

    // Only the usage row's `last_seen` moves: still the session's activity.
    let later = first + 7_200_000 + 1_000;
    grow(&db, 450, 45, later);
    refresh(&mut core);
    assert_eq!(
        events(&core),
        [(first, 100, 10), (seen, 200, 20), (later, 150, 15)]
    );
    assert_eq!(tokens(&mut core), 495);
}

#[test]
fn a_finished_hermes_session_is_counted_once() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), SCHEMA);
    let first = start();
    session(&db, first, 100, 10);
    refresh(&mut core);

    // It grows and ends between two reads.
    let ended = first + 3_600_000 + 1_000;
    grow(&db, 300, 30, ended);
    db.execute(
        "UPDATE sessions SET ended_at=?1,last_activity_at=?1,input_tokens=300,output_tokens=30",
        [seconds(ended)],
    )
    .unwrap();
    refresh(&mut core);
    let counted = [(first, 100, 10), (ended, 200, 20)];
    assert_eq!(events(&core), counted);

    // Read again unchanged, after Hermes touches the finished session, and
    // after a relaunch: the same tokens, never added twice.
    refresh(&mut core);
    db.execute(
        "UPDATE sessions SET last_activity_at=?1",
        [seconds(ended + 60_000)],
    )
    .unwrap();
    refresh(&mut core);
    assert_eq!(events(&core), counted);
    assert_eq!(tokens(&mut core), 330);
    // A relaunch that reads the whole database again from its start, as
    // after a reader update.
    read_from_start(&mut core, root.path());
    drop(core);
    let mut core = backend(root.path());
    refresh(&mut core);
    let path = root.path().join("hermes").join("state.db");
    assert_eq!(
        core.logs[&path].reader_version,
        reader_version(&path, "hermes"),
        "read again from the start"
    );
    assert_eq!(events(&core), counted);
    assert_eq!(tokens(&mut core), 330);
}

/// Makes the next read start over: the database's checkpoint is marked as
/// written by an earlier reader, so its cursor and watermark are dropped.
fn read_from_start(core: &mut AgentHudCore, root: &Path) {
    let path = root.join("hermes").join("state.db");
    let (mut physical, _) = core.load_parser(&path).unwrap();
    physical.reader_version = 1;
    let updated = core.next_timestamp_ms();
    let ledger = core.database.as_mut().unwrap();
    let transaction = ledger.transaction().unwrap();
    checkpoints::upsert_log(
        &transaction,
        &path.to_string_lossy(),
        &serde_json::to_string(&physical).unwrap(),
        &serde_json::to_string(&LogHead::from_log(&physical)).unwrap(),
        updated,
    )
    .unwrap();
    transaction.commit().unwrap();
}

#[test]
fn a_hermes_database_without_last_activity_reads_and_its_growth_counts() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), WITHOUT_LAST_ACTIVITY);
    let first = start();
    session(&db, first, 100, 10);
    refresh(&mut core);
    assert_eq!(tokens(&mut core), 110);

    let seen = first + 3_600_000 + 1_000;
    grow(&db, 300, 30, seen);
    refresh(&mut core);
    assert_eq!(events(&core), [(first, 100, 10), (seen, 200, 20)]);
    assert_eq!(tokens(&mut core), 330);
}

#[test]
fn growth_under_a_stale_or_missing_last_seen_is_dated_at_the_read() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), SCHEMA);
    let first = start();
    session(&db, first, 100, 10);
    refresh(&mut core);

    // Hermes adds tokens but leaves `last_seen` where it was.
    grow(&db, 300, 30, first);
    let before = chrono::Utc::now().timestamp_millis();
    refresh(&mut core);
    let after = chrono::Utc::now().timestamp_millis();
    let read = events(&core);
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], (first, 100, 10));
    assert_eq!((read[1].1, read[1].2), (200, 20));
    assert!((before..=after).contains(&read[1].0), "dated at the read");

    // No stamp at all: the same.
    db.execute(
        "UPDATE session_model_usage SET input_tokens=500,output_tokens=50,last_seen=NULL",
        [],
    )
    .unwrap();
    let before = chrono::Utc::now().timestamp_millis();
    refresh(&mut core);
    let after = chrono::Utc::now().timestamp_millis();
    assert_eq!(tokens(&mut core), 550);
    let grown = events(&core)
        .into_iter()
        .filter(|event| event.0 > first)
        .map(|event| event.0)
        .collect::<Vec<_>>();
    assert!(grown.iter().all(|at| *at <= after));
    assert!(grown.iter().any(|at| *at >= before));
}

#[test]
fn a_late_write_behind_another_sessions_activity_still_counts() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), SCHEMA);
    let first = start();
    session(&db, first, 100, 10);
    // Another session already worked two hours later.
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at)
         VALUES('s2','cli','claude-sonnet-4-5',?1,?2)",
        [seconds(first), seconds(first + 7_200_000)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage(session_id,model,input_tokens,output_tokens,first_seen,last_seen)
         VALUES('s2','claude-sonnet-4-5',5,5,?1,?2)",
        [seconds(first), seconds(first + 7_200_000)],
    )
    .unwrap();
    refresh(&mut core);
    assert_eq!(tokens(&mut core), 110);

    // The first session's call lands now, stamped before the other's
    // activity the last read went past.
    let late = first + 3_600_000 + 1_000;
    db.execute(
        "UPDATE session_model_usage SET input_tokens=200,output_tokens=20,last_seen=?1
         WHERE session_id='s1'",
        [seconds(late)],
    )
    .unwrap();
    refresh(&mut core);
    assert_eq!(tokens(&mut core), 220);
    assert!(events(&core).contains(&(late, 100, 10)));
}

#[test]
fn a_usage_row_first_read_at_zero_dates_its_first_tokens_when_they_come() {
    let root = tempfile::tempdir().unwrap();
    let (db, mut core) = hermes(root.path(), SCHEMA);
    let first = start();
    session(&db, first, 0, 0);
    refresh(&mut core);
    assert!(events(&core).is_empty(), "a zero row makes no event");

    let seen = first + 3_600_000 + 1_000;
    grow(&db, 100, 0, seen);
    refresh(&mut core);
    assert_eq!(events(&core), [(seen, 100, 0)]);
    assert_eq!(tokens(&mut core), 100);
}
