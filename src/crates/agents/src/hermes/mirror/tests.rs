use super::*;
use crate::hermes::gateway::machine_key;
use crate::hermes::transcript::read;
use rusqlite::types::Value as Sql;
use serde_json::Value;

/// A made-up far machine: what a made-up install id makes at a made-up
/// far root home.
fn key() -> MachineKey {
    machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap()
}

fn reports() -> Profile {
    Profile::named("reports").unwrap()
}

/// A mirror of the default profile in a folder of the test's own.
fn mirror(home: &tempfile::TempDir, label: &str) -> (Connection, PathBuf) {
    let path = file(home.path(), &key(), &Profile::Default);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = Connection::open(&path).unwrap();
    assert!(claim(&db, &key(), &Profile::Default, label).unwrap());
    (db, path)
}

fn said(title: &str, model: &str, source: &str, started_at: f64) -> Listed {
    Listed {
        title: Some(title.into()),
        model: Some(model.into()),
        source: Some(source.into()),
        started_at: Some(started_at),
    }
}

/// Every row of every table, as text: what a write may not change.
fn dump(db: &Connection) -> String {
    let mut all = String::new();
    for table in [
        "sessions",
        "session_model_usage",
        "hud_mirror",
        "hud_rows",
        "hud_live",
        "hud_reads",
    ] {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let width = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let cells: Vec<Sql> = (0..width).map(|n| row.get(n).unwrap()).collect();
            all += &format!("{table} {cells:?}\n");
        }
    }
    all
}

fn columns(db: &Connection, table: &str) -> Vec<String> {
    let mut statement = db
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .unwrap();
    let names = statement.query_map([table], |row| row.get(0)).unwrap();
    names.map(Result::unwrap).collect()
}

#[test]
fn a_mirror_is_where_the_shared_walker_lists_a_hermes_database() {
    let data = std::env::temp_dir().join("data");
    let root = data.join(ROOT);
    let path = file(&root, &key(), &reports());
    let folder = format!("{}-reports", key().as_str());
    let expected = ["clients", "hermes-far", "profiles", &folder, "state.db"];
    assert_eq!(path, expected.iter().fold(data, |at, part| at.join(part)));
    assert!(file(&root, &key(), &Profile::Default)
        .ends_with(format!("{}-default/state.db", key().as_str())));
}

#[test]
fn a_claimed_file_has_the_tables_of_the_design_and_is_a_mirror_the_reader_trusts() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    // Spec section 12, as written there.
    assert_eq!(
        columns(&db, "hud_mirror"),
        ["schema", "machine", "label", "profile"]
    );
    assert_eq!(
        columns(&db, "hud_rows"),
        ["session_id", "durable", "read_ms", "print", "held"]
    );
    assert_eq!(
        columns(&db, "hud_live"),
        [
            "session_id",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "read_ms"
        ]
    );
    assert_eq!(
        columns(&db, "hud_reads"),
        [
            "contact_ms",
            "sweep_cursor",
            "sweep_started_ms",
            "sweep_done_ms"
        ]
    );
    // Hermes's two, with every column the reader selects.
    let sessions = columns(&db, "sessions");
    assert_eq!(sessions[0], "id");
    assert_eq!(sessions[1..], SESSION_COLUMNS);
    let usage = columns(&db, "session_model_usage");
    assert_eq!(usage[0], "session_id");
    assert_eq!(usage[1..], USAGE_COLUMNS);
    // One row in each of the two tables that hold one.
    assert_eq!(
        dump(&db),
        format!(
            "hud_mirror [Integer(1), Text(\"{}\"), Text(\"Mac mini\"), Text(\"default\")]\n\
             hud_reads [Null, Null, Null, Null]\n",
            key().as_str()
        )
    );
    let its = identity(&db).unwrap().unwrap();
    assert_eq!(
        (its.key, its.label, its.profile),
        (key().stored(), "Mac mini".to_owned(), Profile::Default)
    );

    // Claimed again as it is: nothing is written.
    let before = dump(&db);
    assert!(!claim(&db, &key(), &Profile::Default, "Mac mini").unwrap());
    assert_eq!(dump(&db), before);
}

#[test]
fn a_listed_session_the_mirror_lacks_gets_a_row_with_no_tokens() {
    let home = tempfile::tempdir().unwrap();
    let (db, path) = mirror(&home, "Mac mini");
    let listed = [
        (
            "s1".to_owned(),
            said("Plan the week", "claude-test", "telegram", 1_760_000_000.5),
        ),
        // Hermes said nothing of it but that it is live.
        ("s2".to_owned(), Listed::default()),
    ];
    assert_eq!(list(&db, &listed, 1_760_000_600_000).unwrap(), 2);

    let k = key();
    let k = k.as_str();
    assert_eq!(
        dump(&db),
        format!(
            "sessions [Text(\"s1\"), Real(1760000000.5), Text(\"Plan the week\"), Null, Text(\"telegram\"), Text(\"claude-test\"), Null, Null, Null, Null, Null, Null, Null, Null, Null, Null]\n\
             sessions [Text(\"s2\"), Real(1760000600.0), Null, Null, Null, Null, Null, Null, Null, Null, Null, Null, Null, Null, Null, Null]\n\
             hud_mirror [Integer(1), Text(\"{k}\"), Text(\"Mac mini\"), Text(\"default\")]\n\
             hud_rows [Text(\"s1\"), Integer(0), Integer(1760000600000), Null, Integer(0)]\n\
             hud_rows [Text(\"s2\"), Integer(0), Integer(1760000600000), Null, Integer(0)]\n\
             hud_reads [Null, Null, Null, Null]\n"
        )
    );

    // The reader lists both as far sessions of that machine, with its
    // label, and counts nothing for either.
    let page = read(&db, &path, &Value::Null).unwrap();
    let shown: Vec<_> = (page.records.iter())
        .map(|r| {
            (
                r.id.clone(),
                r.machine.clone(),
                r.title.clone(),
                r.title_fallback.clone(),
                r.model.clone(),
                r.at,
                r.usage.total(),
            )
        })
        .collect();
    let far = |id: &str| format!("hermes:{k}/default:{id}");
    let label = Some("Mac mini".to_owned());
    assert_eq!(
        shown,
        [
            (
                far("s1"),
                label.clone(),
                Some("Plan the week".into()),
                Some("Hermes · telegram".into()),
                "claude-test".into(),
                1_760_000_000_500,
                0
            ),
            (
                far("s2"),
                label,
                None,
                None,
                "Unknown".into(),
                1_760_000_600_000,
                0
            ),
        ]
    );
}

#[test]
fn a_live_list_write_changes_no_row_that_is_there() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    // A durable row, as the far database gives one: counts, usage, and
    // `durable` 1.
    db.execute_batch(
        "INSERT INTO sessions (id, title, model, source, started_at, input_tokens, output_tokens)
            VALUES ('s1', 'Fix the build', 'claude-test', 'cli', 1760000000, 100, 20);
         INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens, output_tokens, first_seen, last_seen)
            VALUES ('s1', 'claude-test', 'anthropic', 100, 20, 1760000010, 1760000600);
         INSERT INTO hud_rows VALUES ('s1', 1, 1760000700000, 'print-1', 0);",
    )
    .unwrap();
    // And a row a poll made before.
    let first = [(
        "s2".to_owned(),
        said("First title", "m1", "telegram", 1_760_000_100.0),
    )];
    assert_eq!(list(&db, &first, 5_000).unwrap(), 1);
    let before = dump(&db);

    // A later poll lists both, each said otherwise: nothing is written,
    // whether the write is newer or older than the rows.
    for read_ms in [1, 9_999_999_999_999] {
        let again = [
            (
                "s1".to_owned(),
                said("Another title", "other", "telegram", 5.0),
            ),
            ("s2".to_owned(), said("Second title", "m2", "cron", 6.0)),
        ];
        assert_eq!(list(&db, &again, read_ms).unwrap(), 0);
        assert_eq!(dump(&db), before);
    }
    // A durable row whose `hud_rows` entry is all that is left of it is
    // not made again either.
    db.execute("DELETE FROM sessions WHERE id='s1'", [])
        .unwrap();
    let before = dump(&db);
    let again = [("s1".to_owned(), Listed::default())];
    assert_eq!(list(&db, &again, 7_000).unwrap(), 0);
    assert_eq!(dump(&db), before);
}

#[test]
fn a_new_label_is_written_once_and_an_empty_one_never() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    assert!(relabel(&db, &key(), "Studio").unwrap());
    assert!(!relabel(&db, &key(), "Studio").unwrap());
    assert_eq!(identity(&db).unwrap().unwrap().label, "Studio");
    // Claiming under another label writes it; under the same, nothing.
    assert!(claim(&db, &key(), &Profile::Default, "Lab").unwrap());
    assert!(!claim(&db, &key(), &Profile::Default, "Lab").unwrap());
    let before = dump(&db);
    assert!(relabel(&db, &key(), "").is_err());
    assert!(claim(&db, &key(), &Profile::Default, "").is_err());
    assert_eq!(dump(&db), before);
    // A file is never made a mirror nobody could read.
    let empty = Connection::open_in_memory().unwrap();
    assert!(claim(&empty, &key(), &Profile::Default, "").is_err());
    assert!(columns(&empty, "hud_mirror").is_empty());
}

#[test]
fn a_mirror_of_another_machine_or_profile_is_not_claimed() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    let before = dump(&db);
    let other = machine_key(
        Some("fedcba9876543210fedcba9876543210"),
        "/Users/USER/.hermes",
    )
    .unwrap();
    assert!(claim(&db, &other, &Profile::Default, "Mac mini").is_err());
    assert!(claim(&db, &key(), &reports(), "Mac mini").is_err());
    // Nor is it called anything by another machine's writer.
    assert!(relabel(&db, &other, "Studio").is_err());
    assert_eq!(dump(&db), before);
    // A database that is no mirror is given no label.
    let plain = Connection::open_in_memory().unwrap();
    assert!(relabel(&plain, &key(), "Studio").is_err());
    // Nor one whose row the reader would refuse.
    db.execute("UPDATE hud_mirror SET schema=2", []).unwrap();
    assert!(claim(&db, &key(), &Profile::Default, "Mac mini").is_err());
    // Nor Hermes's own database, which has no such table: it is made a
    // mirror only where it is empty of one, and this is never asked of a
    // file under a Hermes home.
}

#[test]
fn the_mirrors_of_a_machine_are_found_whatever_profile_each_is_of() {
    let home = tempfile::tempdir().unwrap();
    let other = machine_key(
        Some("fedcba9876543210fedcba9876543210"),
        "/Users/USER/.hermes",
    )
    .unwrap();
    assert!(files(home.path(), &key()).is_empty());
    for (key, profile) in [
        (key(), Profile::Default),
        (key(), reports()),
        (other.clone(), Profile::Default),
    ] {
        let path = file(home.path(), &key, &profile);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
        std::fs::write(path, "").unwrap();
    }
    // A folder of the machine with no database in it is no mirror.
    std::fs::create_dir_all(
        home.path()
            .join(format!("profiles/{}-empty", key().as_str())),
    )
    .unwrap(); // plain directory
    assert_eq!(
        files(home.path(), &key()),
        [
            file(home.path(), &key(), &Profile::Default),
            file(home.path(), &key(), &reports())
        ]
    );
    assert_eq!(
        files(home.path(), &other),
        [file(home.path(), &other, &Profile::Default)]
    );
}

#[path = "live_tests.rs"]
mod live;
