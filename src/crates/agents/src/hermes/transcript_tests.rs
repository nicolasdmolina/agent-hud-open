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

/// What the reviews of the machine identity found at the reader
/// (`evidence/22/verifier-report.md`, finding 3, and `verifier-report-2.md`,
/// finding 1): a folder or a row could name a far machine's session, and
/// then a root in a folder that is no profile's was not read at all.
#[test]
fn a_folder_or_a_row_that_hermes_would_not_write_names_no_far_session() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    // A folder under `profiles` that is no profile's is a root of its own:
    // read, as its default profile, and named by nothing in its path.
    for folder in [
        "Work",
        "a:b",
        "default",
        ".deleted",
        "19dc3a759af7f225:default",
    ] {
        let path = home.join("profiles").join(folder).join("state.db");
        assert_eq!(
            labels(&path, None),
            [("hermes:s1".to_owned(), Some("Hermes Agent".to_owned()))],
            "{folder}"
        );
    }
    // A row named as a far session is left out; the rows beside it stay.
    let path = home.join("state.db");
    let db = state_db(&path, None);
    for id in ["19dc3a759af7f225/default:s1", "a/b", "agent:main:x/y"] {
        db.execute(
            "INSERT INTO sessions (id, source, started_at) VALUES (?1, 'cli', 1760000000)",
            [id],
        )
        .unwrap();
    }
    let page = read(&db, &path, &Value::Null).unwrap();
    let mut ids: Vec<&str> = page
        .records
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    ids.dedup();
    assert_eq!(ids, ["hermes:s1", "hermes:agent:main:x/y"]);
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

/// A far machine's key, as a mirror's writer stores it (made up).
const KEY: &str = "aaeda597d860920c";

/// Makes `db` a mirror: the `hud_mirror` table, with one row of these
/// values, typed as given.
fn mirror(db: &Connection, row: [rusqlite::types::Value; 4]) {
    db.execute_batch(
        "CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO hud_mirror VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params_from_iter(row),
    )
    .unwrap();
}

fn row(schema: i64, machine: &str, label: &str, profile: &str) -> [rusqlite::types::Value; 4] {
    use rusqlite::types::Value::{Integer, Text};
    [
        Integer(schema),
        Text(machine.into()),
        Text(label.into()),
        Text(profile.into()),
    ]
}

/// What a record counts and is shown as, with its id taken out of its key.
fn counted(record: &Record) -> String {
    format!(
        "{} {:?} {:?} {:?} {} {} {:?} {:?} {:?} {:?}",
        record.key.replacen(&record.id, "<id>", 1),
        record.title,
        record.title_fallback,
        record.cwd,
        record.model,
        record.at,
        record.usage,
        record.running,
        record.updated,
        record.archived,
    )
}

#[test]
fn a_mirror_names_far_sessions_and_counts_what_the_same_rows_count() {
    let home = tempfile::tempdir().unwrap();
    let plain_path = home.path().join("plain/state.db");
    let plain = read(&state_db(&plain_path, None), &plain_path, &Value::Null).unwrap();
    let path = home.path().join(format!("profiles/{KEY}-default/state.db"));
    let db = state_db(&path, None);
    mirror(&db, row(1, KEY, "Mac mini", "default"));
    let far = read(&db, &path, &Value::Null).unwrap();

    // The same database without the table reads as it always has.
    let ids = |page: &Page| {
        page.records
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&plain), ["hermes:s1"]);
    assert!(plain.records.iter().all(|r| r.machine.is_none()));
    // The mirror's sessions are far ones, named by the table's machine.
    assert_eq!(ids(&far), [format!("hermes:{KEY}/default:s1")]);
    for record in &far.records {
        assert_eq!(record.machine.as_deref(), Some("Mac mini"));
        assert_eq!(record.client, None);
    }
    // And they count exactly what the same rows count on this machine.
    assert_eq!(
        far.records.iter().map(counted).collect::<Vec<_>>(),
        plain.records.iter().map(counted).collect::<Vec<_>>()
    );
}

#[test]
fn a_mirror_takes_its_profile_from_its_table_and_never_its_folder() {
    let home = tempfile::tempdir().unwrap();
    // `profiles/work` is where this machine keeps its profile `work`.
    for (n, (profile, id, client)) in [
        ("default", format!("hermes:{KEY}/default:s1"), None),
        (
            "reports",
            format!("hermes:{KEY}/reports:s1"),
            Some("reports"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = home.path().join(format!("{n}/profiles/work/state.db"));
        let db = state_db(&path, None);
        mirror(&db, row(1, KEY, "Mac mini", profile));
        let page = read(&db, &path, &Value::Null).unwrap();
        let records = page
            .records
            .iter()
            .map(|r| (r.id.clone(), r.client.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(records, [(id, client)], "{profile}");
    }
}

#[test]
fn a_new_label_reads_every_session_again_and_nothing_else_does() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(format!("profiles/{KEY}-default/state.db"));
    let db = state_db(&path, None);
    db.execute(
        "INSERT INTO sessions (id, source, started_at, input_tokens) VALUES ('s2', 'telegram', 1760000100, 5)",
        [],
    )
    .unwrap();
    mirror(&db, row(1, KEY, "Mac mini", "default"));
    let first = read(&db, &path, &Value::Null).unwrap();
    let watermark = first.watermark.unwrap();
    assert_eq!(first.records.len(), 2);
    // Nothing changed: nothing is read again.
    let again = read(&db, &path, &watermark).unwrap();
    assert!(again.records.is_empty());

    db.execute("UPDATE hud_mirror SET label='Studio'", [])
        .unwrap();
    let renamed = read(&db, &path, &watermark).unwrap();
    assert_eq!(
        renamed.records.iter().map(|r| &r.key).collect::<Vec<_>>(),
        first.records.iter().map(|r| &r.key).collect::<Vec<_>>()
    );
    assert!(renamed
        .records
        .iter()
        .all(|r| r.machine.as_deref() == Some("Studio")));
    assert_eq!(
        renamed.records.iter().map(counted).collect::<Vec<_>>(),
        first.records.iter().map(counted).collect::<Vec<_>>()
    );
    let after = read(&db, &path, &renamed.watermark.unwrap()).unwrap();
    assert!(after.records.is_empty());
}

#[test]
fn a_mirror_that_cannot_be_trusted_is_refused_whole() {
    use rusqlite::types::Value::{Integer, Null, Text};
    let text = |s: &str| Text(s.into());
    let cases: Vec<(&str, Vec<[rusqlite::types::Value; 4]>)> = vec![
        ("schema 2", vec![row(2, KEY, "Mac mini", "default")]),
        ("schema 0", vec![row(0, KEY, "Mac mini", "default")]),
        (
            "schema null",
            vec![[Null, text(KEY), text("Mac mini"), text("default")]],
        ),
        (
            "schema text",
            vec![[text("one"), text(KEY), text("Mac mini"), text("default")]],
        ),
        (
            "machine 15",
            vec![row(1, &KEY[..15], "Mac mini", "default")],
        ),
        (
            "machine 17",
            vec![row(1, &format!("{KEY}0"), "Mac mini", "default")],
        ),
        (
            "machine upper",
            vec![row(1, &KEY.to_uppercase(), "Mac mini", "default")],
        ),
        (
            "machine not hex",
            vec![row(1, "aaeda597d860920g", "Mac mini", "default")],
        ),
        (
            "machine install id",
            vec![row(
                1,
                "0123456789abcdef0123456789abcdef",
                "Mac mini",
                "default",
            )],
        ),
        (
            "machine null",
            vec![[Integer(1), Null, text("Mac mini"), text("default")]],
        ),
        ("label empty", vec![row(1, KEY, "", "default")]),
        (
            "label null",
            vec![[Integer(1), text(KEY), Null, text("default")]],
        ),
        ("profile empty", vec![row(1, KEY, "Mac mini", "")]),
        ("profile upper", vec![row(1, KEY, "Mac mini", "Work")]),
        ("profile slash", vec![row(1, KEY, "Mac mini", "a/b")]),
        (
            "profile null",
            vec![[Integer(1), text(KEY), text("Mac mini"), Null]],
        ),
        ("no row", vec![]),
        (
            "two rows",
            vec![
                row(1, KEY, "Mac mini", "default"),
                row(1, KEY, "Mac mini", "default"),
            ],
        ),
    ];
    let home = tempfile::tempdir().unwrap();
    for (n, (case, rows)) in cases.into_iter().enumerate() {
        let path = home
            .path()
            .join(format!("{n}/profiles/{KEY}-default/state.db"));
        let db = state_db(&path, None);
        db.execute_batch(
            "CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);",
        )
        .unwrap();
        for row in rows {
            db.execute(
                "INSERT INTO hud_mirror VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params_from_iter(row),
            )
            .unwrap();
        }
        // Refused on the first page and on any later one.
        for cursor in [Value::Null, json!({"scan": 1}), json!({"rowid_after": 9})] {
            assert!(read(&db, &path, &cursor).is_err(), "{case} {cursor}");
        }
    }
}
