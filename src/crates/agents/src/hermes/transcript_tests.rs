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
