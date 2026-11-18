//! The durable write of a mirror (ticket 37), on files of the tests' own.
//! "Far" is a database with Hermes's columns made here, read as the far
//! reader gives it: each session's cells in the order of the column
//! lists, with the fingerprint `local_prints` makes.
use super::*;
use crate::hermes::far_history::local_prints;
use crate::hermes::gateway::{machine_key, Listed, MachineKey, Profile};
use crate::hermes::mirror::{claim, file, list};
use crate::hermes::transcript::read;
use rusqlite::types::ValueRef;
use serde_json::Value;

fn key() -> MachineKey {
    machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap()
}

/// A far `state.db` with Hermes's columns, in a folder of the test's own.
fn far(home: &tempfile::TempDir) -> Connection {
    let db = Connection::open(home.path().join("state.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, started_at REAL NOT NULL,
            ended_at REAL, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0, cwd TEXT,
            billing_provider TEXT, billing_base_url TEXT, billing_mode TEXT, title TEXT,
            last_activity_at REAL, profile_name TEXT);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL,
            billing_provider TEXT NOT NULL DEFAULT '', billing_base_url TEXT NOT NULL DEFAULT '',
            billing_mode TEXT NOT NULL DEFAULT '', task TEXT NOT NULL DEFAULT '',
            input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            first_seen REAL, last_seen REAL,
            PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task));",
    )
    .unwrap();
    db
}

/// A far session that ended, with its own counts.
fn session(db: &Connection, id: &str, input: i64) {
    db.execute(
        "INSERT INTO sessions VALUES (?1, 'telegram', 'claude-test', 1760000000.5, 1760000900.25, ?2, 20,
            300, 4, '/Users/far/work', 'anthropic', 'https://api.anthropic.com', 'api', 'Made-up title',
            1760000600.75, NULL)",
        params![id, input],
    )
    .unwrap();
}

/// One usage row of `id`: by `model` through OpenRouter, for `task`.
fn used(db: &Connection, id: &str, model: &str, task: &str, input: i64) {
    db.execute(
        "INSERT INTO session_model_usage VALUES (?1, ?2, 'openrouter', 'https://openrouter.ai/api/v1', 'api', ?3,
            ?4, 7, 11, 13, 1760000010.125, 1760000500.5)",
        params![id, model, task, input],
    )
    .unwrap();
}

fn cell(value: ValueRef<'_>) -> Cell {
    match value {
        ValueRef::Null | ValueRef::Blob(_) => Cell::Null,
        ValueRef::Integer(whole) => Cell::Int(whole),
        ValueRef::Real(number) => Cell::Real(number),
        ValueRef::Text(text) => Cell::Text(String::from_utf8_lossy(text).into_owned()),
    }
}

/// The session `id` of the far database as the far reader gives it.
fn given(db: &Connection, id: &str) -> Session {
    let cells = |sql: String, width: usize| {
        let mut statement = db.prepare(&sql).unwrap();
        let rows = statement.query_map([id], |row| {
            Ok((0..width).map(|at| cell(row.get_ref_unwrap(at))).collect())
        });
        rows.unwrap()
            .map(Result::unwrap)
            .collect::<Vec<Vec<Cell>>>()
    };
    let own = format!(
        "SELECT {} FROM sessions WHERE id=?1",
        SESSION_COLUMNS.join(",")
    );
    let usage = format!(
        "SELECT {} FROM session_model_usage WHERE session_id=?1",
        USAGE_COLUMNS.join(",")
    );
    let prints = local_prints(db, 0, 500).unwrap();
    let print = prints.into_iter().find(|print| print.id == id).unwrap();
    Session {
        rowid: print.rowid,
        id: id.to_owned(),
        print: print.print,
        cells: cells(own, SESSION_COLUMNS.len()).remove(0),
        usage: cells(usage, USAGE_COLUMNS.len()),
    }
}

/// A mirror of the default profile in a folder of the test's own.
fn mirror(home: &tempfile::TempDir) -> (Connection, std::path::PathBuf) {
    let path = file(&home.path().join("mirrors"), &key(), &Profile::Default);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = Connection::open(&path).unwrap();
    assert!(claim(&db, &key(), &Profile::Default, "Mac mini").unwrap());
    (db, path)
}

/// Every row of `tables`, as text.
fn dump(db: &Connection, tables: &[&str]) -> String {
    let mut all = String::new();
    for table in tables {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
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

/// What the reader makes of a whole database: each record's session, key
/// and counts, the session written without what names its machine.
fn records(db: &Connection, path: &std::path::Path) -> Vec<(String, String, [u64; 4])> {
    let (mut cursor, mut all) = (Value::Null, Vec::new());
    for _ in 0..50 {
        let page = read(db, path, &cursor).unwrap();
        all.extend(page.records);
        match page.cursor {
            Some(next) => cursor = next,
            None => break,
        }
    }
    let bare = |text: &str| text.replace(&format!("{}/default:", key().as_str()), "");
    let mut all: Vec<(String, String, [u64; 4])> = (all.iter())
        .map(|record| {
            let used = &record.usage;
            let counts = [used.input, used.output, used.read, used.write];
            (bare(&record.id), bare(&record.key), counts)
        })
        .collect();
    all.sort();
    all
}

#[test]
fn a_sessions_durable_rows_are_written_whole_and_the_reader_reads_them_as_the_far_database() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    session(&far, "s-used", 1_000);
    used(&far, "s-used", "model-a", "", 100);
    used(&far, "s-used", "model-b", "", 60);
    used(&far, "s-used", "model-a", "aux:title", 5);
    session(&far, "s-bare", 2_000);
    let (db, path) = mirror(&home);
    let both = [given(&far, "s-used"), given(&far, "s-bare")];
    assert_eq!(durable(&db, &both, 5_000).unwrap(), 2);

    // Field for field what the far database holds, in its column order.
    for id in ["s-used", "s-bare"] {
        let mut stored = given(&db, id);
        let mut asked = given(&far, id);
        stored.usage.sort_by_key(|row| format!("{row:?}"));
        asked.usage.sort_by_key(|row| format!("{row:?}"));
        assert_eq!(
            (stored.cells, stored.usage),
            (asked.cells, asked.usage),
            "{id}"
        );
    }
    // Each is durable, with the fingerprint the far reader gave.
    let marked = dump(&db, &["hud_rows"]);
    for session in &both {
        let row = format!(
            "hud_rows [Text({:?}), Integer(1), Integer(5000), Text({:?}), Integer(0)]",
            session.id, session.print
        );
        assert!(marked.contains(&row), "{marked}");
    }
    assert_eq!(stored(&db).unwrap().len(), 2);
    assert_eq!(stored(&db).unwrap()["s-used"], both[0].print);

    // And the reader makes of the mirror what it makes of the far
    // database itself: the same records, the task in each one's key.
    let direct = records(&far, &home.path().join("state.db"));
    assert_eq!(records(&db, &path), direct);
    assert_eq!(direct.len(), 4);
    let aux = (direct.iter()).find(|(_, key, _)| key.contains("aux:title"));
    assert_eq!(aux.unwrap().2, [5, 7, 11, 13]);
}

#[test]
fn a_row_the_live_list_made_is_replaced_in_place_and_keeps_its_rowid() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    for id in ["s-live", "s-newest"] {
        session(&far, id, 1_000);
        used(&far, id, "model-a", "", 100);
    }
    let (db, _) = mirror(&home);
    let said = Listed {
        title: Some("From the live list".into()),
        ..Listed::default()
    };
    let listed: Vec<(String, Listed)> = ["s-live", "s-older", "s-newest"]
        .map(|id| (id.to_owned(), said.clone()))
        .into();
    assert_eq!(list(&db, &listed, 1_000).unwrap(), 3);
    let rowids = |db: &Connection| -> Vec<(String, i64)> {
        let mut statement = db
            .prepare("SELECT id, rowid FROM sessions ORDER BY rowid")
            .unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)));
        rows.unwrap().map(Result::unwrap).collect()
    };
    let before = rowids(&db);
    assert_eq!(before[2], ("s-newest".to_owned(), 3));
    // Read from the database earlier than the live list listed them: a
    // row made from the live list is replaced whenever it was made. One
    // is the mirror's newest row, which must never be deleted, and one
    // is not: a row deleted and put in again would get a new rowid.
    let both = [given(&far, "s-live"), given(&far, "s-newest")];
    assert_eq!(durable(&db, &both, 500).unwrap(), 2);
    assert_eq!(rowids(&db), before);
    assert_eq!(given(&db, "s-live").cells, given(&far, "s-live").cells);
    let rows = dump(&db, &["hud_rows"]);
    for id in ["s-live", "s-newest"] {
        let marked = format!("hud_rows [Text({id:?}), Integer(1), Integer(500), ");
        assert!(rows.contains(&marked), "{rows}");
    }
    let untouched = "hud_rows [Text(\"s-older\"), Integer(0), Integer(1000), Null, ";
    assert!(rows.contains(untouched), "{rows}");
    // A live list that names them again changes nothing.
    assert_eq!(list(&db, &listed, 2_000).unwrap(), 0);
    assert_eq!(rowids(&db), before);
}

#[test]
fn a_batch_read_before_the_one_stored_is_dropped_and_the_same_one_twice_writes_once() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    session(&far, "s-one", 1_000);
    used(&far, "s-one", "model-a", "", 100);
    let (db, _) = mirror(&home);
    let old = given(&far, "s-one");
    far.execute("UPDATE session_model_usage SET input_tokens=250", [])
        .unwrap();
    used(&far, "s-one", "model-a", "aux:compress", 9);
    let new = given(&far, "s-one");
    assert_ne!(old.print, new.print);

    assert_eq!(durable(&db, std::slice::from_ref(&new), 2_000).unwrap(), 1);
    let held = dump(&db, &["sessions", "session_model_usage", "hud_rows"]);
    // The older read comes back late: nothing of it is written.
    assert_eq!(durable(&db, std::slice::from_ref(&old), 1_000).unwrap(), 0);
    // And the newer one once more.
    assert_eq!(durable(&db, std::slice::from_ref(&new), 2_000).unwrap(), 0);
    assert_eq!(
        dump(&db, &["sessions", "session_model_usage", "hud_rows"]),
        held
    );
    assert_eq!(given(&db, "s-one").usage.len(), 2);

    // A read after it replaces every usage row: none is added twice,
    // and one that is gone from the far database is gone here.
    far.execute(
        "DELETE FROM session_model_usage WHERE task='aux:compress'",
        [],
    )
    .unwrap();
    assert_eq!(durable(&db, &[given(&far, "s-one")], 3_000).unwrap(), 1);
    assert_eq!(given(&db, "s-one").usage, given(&far, "s-one").usage);
    assert_eq!(given(&db, "s-one").usage.len(), 1);
}

#[test]
fn a_batch_changes_nothing_of_the_live_counts() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    session(&far, "s-one", 1_000);
    let (db, _) = mirror(&home);
    db.execute(
        "INSERT INTO hud_live VALUES ('s-one', 900, 80, 7, 6, 4000)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO hud_live VALUES ('s-two', 1, 2, 3, 4, 4000)",
        [],
    )
    .unwrap();
    let before = dump(&db, &["hud_live"]);
    assert_eq!(durable(&db, &[given(&far, "s-one")], 2_000).unwrap(), 1);
    assert_eq!(held(&db, &["s-one".to_owned()]).unwrap(), 1);
    let ages = Reads {
        contact_ms: Some(1),
        ..Reads::default()
    };
    assert_eq!(note(&db, &ages).unwrap(), 1);
    assert_eq!(dump(&db, &["hud_live"]), before);
}

#[test]
fn a_session_of_the_wrong_shape_is_an_error_before_its_row_is_touched() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    session(&far, "s-one", 1_000);
    used(&far, "s-one", "model-a", "", 100);
    let (db, _) = mirror(&home);
    let mut short = given(&far, "s-one");
    short.usage[0].pop();
    assert!(durable(&db, &[short], 2_000).is_err());
    let mut long = given(&far, "s-one");
    long.cells.push(Cell::Null);
    assert!(durable(&db, &[long], 2_000).is_err());
    assert_eq!(
        dump(&db, &["sessions", "session_model_usage", "hud_rows"]),
        ""
    );
}

#[test]
fn the_sessions_the_desktop_app_holds_are_marked_and_unmarked() {
    let home = tempfile::tempdir().unwrap();
    let far = far(&home);
    for id in ["s-a", "s-b", "s-c"] {
        session(&far, id, 1_000);
    }
    let (db, _) = mirror(&home);
    let all: Vec<Session> = ["s-a", "s-b", "s-c"].map(|id| given(&far, id)).into();
    durable(&db, &all, 2_000).unwrap();
    let marked = |db: &Connection| -> Vec<String> {
        let mut statement = db
            .prepare("SELECT session_id FROM hud_rows WHERE held<>0 ORDER BY 1")
            .unwrap();
        let rows = statement.query_map([], |row| row.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
    // One the mirror has no row for is not given one.
    assert_eq!(held(&db, &ids(&["s-a", "s-b", "s-nowhere"])).unwrap(), 2);
    assert_eq!(marked(&db), ["s-a", "s-b"]);
    assert_eq!(held(&db, &ids(&["s-a", "s-b"])).unwrap(), 0);
    assert_eq!(held(&db, &ids(&["s-b", "s-c"])).unwrap(), 2);
    assert_eq!(marked(&db), ["s-b", "s-c"]);
    // A durable write leaves the mark as it is.
    durable(&db, &all, 3_000).unwrap();
    assert_eq!(marked(&db), ["s-b", "s-c"]);
    assert_eq!(held(&db, &[]).unwrap(), 2);
    assert!(marked(&db).is_empty());
}

#[test]
fn the_two_ages_and_the_sweeps_place_are_kept() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home);
    assert_eq!(reads(&db).unwrap(), Reads::default());
    let ages = Reads {
        contact_ms: Some(9_000),
        sweep_cursor: Some(1_500),
        sweep_started_ms: Some(8_000),
        sweep_done_ms: None,
    };
    assert_eq!(note(&db, &ages).unwrap(), 1);
    assert_eq!(note(&db, &ages).unwrap(), 0);
    assert_eq!(reads(&db).unwrap(), ages);
    let done = Reads {
        sweep_cursor: None,
        sweep_done_ms: Some(9_500),
        ..ages
    };
    assert_eq!(note(&db, &done).unwrap(), 1);
    assert_eq!(reads(&db).unwrap(), done);
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM hud_reads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}
