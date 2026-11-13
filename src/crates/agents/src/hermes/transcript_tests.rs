use super::*;
use crate::naming::clients::client_name;

/// A `state.db` with the columns the reader reads, spelled as Hermes spells
/// them (Open's `HermesProviderTests.stateDatabase`), holding session `s1`
/// whose `profile_name` is `profile`.
fn state_db(path: &Path, profile: Option<&str>) -> Connection {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, started_at REAL NOT NULL, ended_at REAL, input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0, cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0, cwd TEXT, billing_provider TEXT,
            billing_base_url TEXT, billing_mode TEXT, title TEXT, last_activity_at REAL, profile_name TEXT);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL, billing_provider TEXT NOT NULL DEFAULT '', billing_base_url TEXT NOT NULL DEFAULT '',
            billing_mode TEXT NOT NULL DEFAULT '', task TEXT NOT NULL DEFAULT '', input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0, first_seen REAL, last_seen REAL,
            PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task));
         INSERT INTO session_model_usage VALUES ('s1', 'claude-test', 'anthropic', '', 'api', '', 100, 20, 50, 10, 1760000010, 1760000600);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES ('s1', 'cli', 'claude-test', 1760000000, NULL, 999, 999, 999, 999, '/work/app', 'anthropic', '', 'api', 'Fix the build', 1760000600, ?1)",
        [profile],
    )
    .unwrap();
    db
}

/// Each session's id and the client it is shown by.
fn labels(path: &Path, profile: Option<&str>) -> Vec<(String, Option<String>)> {
    let db = state_db(path, profile);
    read(&db, path, &Value::Null)
        .unwrap()
        .records
        .into_iter()
        .map(|record| {
            let shown = client_name("hermes", record.client.as_deref());
            (record.id, shown)
        })
        .collect()
}

#[test]
fn sessions_are_labelled_by_their_profile() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let shown = |id: &str, client: &str| vec![(id.to_owned(), Some(client.to_owned()))];
    // Open's `testProfileDatabasesAreScopedAndSourceCheckoutIsNotScanned`.
    assert_eq!(
        labels(&home.join("state.db"), None),
        shown("hermes:s1", "Hermes Agent")
    );
    assert_eq!(
        labels(&home.join("profiles/reports/state.db"), Some("reports")),
        shown("hermes:reports:s1", "Hermes Agent · reports")
    );
    // The default profile names itself; it is still the default.
    assert_eq!(
        labels(&home.join("default/state.db"), Some("default")),
        shown("hermes:s1", "Hermes Agent")
    );
    // A session that names no profile is named by its database's folder.
    assert_eq!(
        labels(&home.join("profiles/coder/state.db"), None),
        shown("hermes:coder:s1", "Hermes Agent · coder")
    );
}

const SCHEMA: &str = include_str!("../../tests/fixtures/hermes/macos/state.schema.sql");

/// A real-schema `state.db` in memory with `count` sessions, one usage row
/// each, all last active at the same time.
fn sessions(count: i64) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(SCHEMA).unwrap();
    for n in 1..=count {
        db.execute(
            "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES(?1,'cli','m',1760000000,1760000600)",
            [format!("s{n}")],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_model_usage(session_id,model,input_tokens,first_seen,last_seen) VALUES(?1,'m',10,1760000000,1760000600)",
            [format!("s{n}")],
        )
        .unwrap();
    }
    db
}

/// One whole pass from `cursor`, as core reads it: the records' session ids,
/// and the watermark it ends with. Each page carries one map of counts and
/// the first page drops the last pass's watermark.
fn pass(db: &Connection, cursor: Value) -> (Vec<String>, Value) {
    let path = Path::new("state.db");
    let (mut cursor, mut ids, mut first) = (cursor, Vec::new(), true);
    loop {
        let page = read(db, path, &cursor).unwrap();
        ids.extend(page.records.into_iter().map(|record| record.id));
        let Some(next) = page.cursor else {
            return (ids, page.watermark.unwrap());
        };
        assert!(next["next"].is_null(), "one map, not two");
        assert_eq!(page.watermark, first.then_some(Value::Null));
        first = false;
        cursor = next;
    }
}

#[test]
fn a_pass_keeps_one_map_of_counts_and_rereads_a_session_that_grew_behind_the_watermark() {
    let db = sessions(250);
    let (ids, watermark) = pass(&db, Value::Null);
    assert_eq!(ids.len(), 250);
    assert_eq!(watermark["prints"].as_object().unwrap().len(), 250);
    let (ids, watermark) = pass(&db, watermark);
    assert!(ids.is_empty(), "nothing changed");

    // A call stamped no later than the watermark, and a session removed.
    db.execute_batch(
        "UPDATE session_model_usage SET input_tokens=20 WHERE session_id='s150';
         DELETE FROM session_model_usage WHERE session_id='s7'; DELETE FROM sessions WHERE id='s7';",
    )
    .unwrap();
    let (ids, watermark) = pass(&db, watermark);
    assert_eq!(ids, ["hermes:s150"]);
    let prints = watermark["prints"].as_object().unwrap();
    assert_eq!(prints.len(), 249);
    assert!(!prints.contains_key("7"));
}
