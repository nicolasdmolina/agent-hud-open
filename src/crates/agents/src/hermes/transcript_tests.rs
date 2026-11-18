use super::*;
use crate::hermes::gateway::Profile;
use crate::naming::clients::client_name;
use crate::transcript::records::Record;

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

/// The most pages a pass over `db` may take: one for every hundred sessions
/// it holds, read as new and again as old, and two more. A reader that
/// stops moving fails a test here instead of looping in it without end
/// (ticket 54's verifier, finding 4).
fn most_pages(db: &Connection) -> usize {
    let count = db.query_row("SELECT COUNT(*) FROM sessions", [], |row| {
        row.get::<_, i64>(0)
    });
    2 * (count.unwrap() as usize / 100 + 2)
}

/// One whole pass from `cursor`, as core reads it: the records' session ids,
/// and the watermark it ends with. Each page carries one map of counts and
/// the first page drops the last pass's watermark.
fn pass(db: &Connection, cursor: Value) -> (Vec<String>, Value) {
    let path = Path::new("state.db");
    let (mut cursor, mut ids, mut first) = (cursor, Vec::new(), true);
    for _ in 0..most_pages(db) {
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
    panic!("the pass did not end in {} pages", most_pages(db));
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

/// How a test's mirror came to be: by hand, the `hud_mirror` table added
/// to a database in Hermes's own schema (ticket 23's fixture), or by the
/// mirror writer's own code (`mirror::claim`), with the rows the far
/// database would give put in after.
#[derive(Clone, Copy, Debug)]
enum Made {
    ByHand,
    Written,
}

const MADE: [Made; 2] = [Made::ByHand, Made::Written];

/// A mirror at `path` of the far profile `profile` of the machine `KEY`,
/// shown as `label`, holding session `s1` as `state_db` holds it.
fn far_db(path: &Path, made: Made, label: &str, profile: &str) -> Connection {
    match made {
        Made::ByHand => {
            let db = state_db(path, None);
            mirror(&db, row(1, KEY, label, profile));
            db
        }
        Made::Written => {
            let key = super::super::gateway::machine_key(
                Some("0123456789abcdef0123456789abcdef"),
                "/Users/USER/.hermes",
            )
            .unwrap();
            assert_eq!(key.as_str(), KEY);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
            let db = Connection::open(path).unwrap();
            let of = Profile::named(profile).unwrap();
            assert!(super::super::mirror::claim(&db, &key, &of, label).unwrap());
            db.execute_batch(
                "INSERT INTO session_model_usage VALUES ('s1', 'claude-test', 'anthropic', '', 'api', '', 100, 20, 50, 10, 1760000010, 1760000600);
                 INSERT INTO sessions (id, source, model, started_at, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cwd, billing_provider, billing_base_url, billing_mode, title, last_activity_at)
                    VALUES ('s1', 'cli', 'claude-test', 1760000000, 999, 999, 999, 999, '/work/app', 'anthropic', '', 'api', 'Fix the build', 1760000600);
                 INSERT INTO hud_rows VALUES ('s1', 1, 1760000700000, 'print', 0);",
            )
            .unwrap();
            db
        }
    }
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
    // The same database without the table reads as it always has.
    let ids = |page: &Page| {
        page.records
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&plain), ["hermes:s1"]);
    assert!(plain.records.iter().all(|r| r.machine.is_none()));
    // By hand, and as the mirror writer's own code makes one.
    for made in MADE {
        let path = home
            .path()
            .join(format!("{made:?}/profiles/{KEY}-default/state.db"));
        let db = far_db(&path, made, "Mac mini", "default");
        let far = read(&db, &path, &Value::Null).unwrap();
        // The mirror's sessions are far ones, named by the table's machine.
        assert_eq!(ids(&far), [format!("hermes:{KEY}/default:s1")], "{made:?}");
        for record in &far.records {
            assert_eq!(record.machine.as_deref(), Some("Mac mini"));
            assert_eq!(record.client, None);
        }
        // And they count exactly what the same rows count on this machine.
        assert_eq!(
            far.records.iter().map(counted).collect::<Vec<_>>(),
            plain.records.iter().map(counted).collect::<Vec<_>>(),
            "{made:?}"
        );
    }
}

#[test]
fn a_mirror_takes_its_profile_from_its_table_and_never_its_folder() {
    let home = tempfile::tempdir().unwrap();
    for made in MADE {
        a_mirror_made(made, &home.path().join(format!("{made:?}")));
    }
}

fn a_mirror_made(made: Made, home: &Path) {
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
        let path = home.join(format!("{n}/profiles/work/state.db"));
        let db = far_db(&path, made, "Mac mini", profile);
        let page = read(&db, &path, &Value::Null).unwrap();
        let records = page
            .records
            .iter()
            .map(|r| (r.id.clone(), r.client.as_deref()))
            .collect::<Vec<_>>();
        assert_eq!(records, [(id, client)], "{profile} {made:?}");
    }
}

#[test]
fn a_new_label_reads_every_session_again_and_nothing_else_does() {
    let home = tempfile::tempdir().unwrap();
    for made in MADE {
        a_new_label_on_a_mirror_made(made, &home.path().join(format!("{made:?}")));
    }
}

fn a_new_label_on_a_mirror_made(made: Made, home: &Path) {
    let path = home.join(format!("profiles/{KEY}-default/state.db"));
    let db = far_db(&path, made, "Mac mini", "default");
    db.execute(
        "INSERT INTO sessions (id, source, started_at, input_tokens) VALUES ('s2', 'telegram', 1760000100, 5)",
        [],
    )
    .unwrap();
    let first = read(&db, &path, &Value::Null).unwrap();
    let watermark = first.watermark.unwrap();
    assert_eq!(first.records.len(), 2);
    // Nothing changed: nothing is read again.
    let again = read(&db, &path, &watermark).unwrap();
    assert!(again.records.is_empty());

    // As a test writes it, and as the writer does.
    match made {
        Made::ByHand => drop(db.execute("UPDATE hud_mirror SET label='Studio'", [])),
        Made::Written => {
            let key = super::super::gateway::machine_key(
                Some("0123456789abcdef0123456789abcdef"),
                "/Users/USER/.hermes",
            )
            .unwrap();
            assert!(super::super::mirror::relabel(&db, &key, "Studio").unwrap());
        }
    }
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

/// One more session in `db`, `s<n>`, with one usage row.
fn begins(db: &Connection, n: i64) {
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES(?1,'cli','m',1760000700,1760000700)",
        [format!("s{n}")],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage(session_id,model,input_tokens,first_seen,last_seen) VALUES(?1,'m',10,1760000700,1760000700)",
        [format!("s{n}")],
    )
    .unwrap();
}

/// The sessions one page reads from `cursor`, and the page.
fn page(db: &Connection, cursor: &Value) -> (Vec<String>, Page) {
    let page = read(db, Path::new("state.db"), cursor).unwrap();
    let ids = page
        .records
        .iter()
        .map(|record| record.id.clone())
        .collect();
    (ids, page)
}

/// The pages from `cursor`, which a pass under way left, to the pass's
/// end: the sessions read, and the watermark it ends with.
fn rest(db: &Connection, cursor: Value) -> (Vec<String>, Value) {
    let (mut cursor, mut all) = (cursor, Vec::new());
    for _ in 0..most_pages(db) {
        let (ids, page) = page(db, &cursor);
        all.extend(ids);
        let Some(next) = page.cursor else {
            return (all, page.watermark.unwrap());
        };
        assert_eq!(page.watermark, None);
        cursor = next;
    }
    panic!("the pass did not end in {} pages", most_pages(db));
}

fn named(sessions: impl IntoIterator<Item = i64>) -> Vec<String> {
    sessions
        .into_iter()
        .map(|n| format!("hermes:s{n}"))
        .collect()
}

/// Ticket 54: a session that begins is read on the first page after it,
/// however many sessions the database holds already. Read last, as it
/// was, it waited one pass for each hundred of them.
#[test]
fn a_new_session_is_read_on_the_first_page_however_many_are_there() {
    for held in [90, 250, 1_000, 5_000] {
        let db = sessions(held);
        let (_, watermark) = pass(&db, Value::Null);
        begins(&db, held + 1);
        let (first, _) = page(&db, &watermark);
        assert_eq!(first, named([held + 1]), "{held} held");
        // The whole pass reads it once, and nothing else.
        let (ids, watermark) = pass(&db, watermark);
        assert_eq!(ids, named([held + 1]), "{held} held");
        assert_eq!(watermark["rowid_after"], held + 1);
        assert_eq!(
            watermark["prints"].as_object().unwrap().len() as i64,
            held + 1
        );
        let (ids, _) = pass(&db, watermark);
        assert!(ids.is_empty(), "{held} held: nothing changed");
    }
}

/// The same while a pass is under way: the page after the session began
/// reads it, wherever the pass has got to.
#[test]
fn a_session_that_begins_while_a_pass_is_under_way_is_read_on_the_next_page() {
    let db = sessions(1_000);
    let (_, watermark) = pass(&db, Value::Null);
    let (ids, first) = page(&db, &watermark);
    assert!(ids.is_empty());
    let mut cursor = first.cursor.unwrap();
    for _ in 0..3 {
        cursor = page(&db, &cursor).1.cursor.unwrap();
    }
    begins(&db, 1_001);
    let (ids, next) = page(&db, &cursor);
    assert_eq!(ids, named([1_001]));
    // A session whose counts grew behind the pass is still found by it,
    // and the new one is not read a second time.
    db.execute_batch("UPDATE session_model_usage SET input_tokens=20 WHERE session_id='s950'")
        .unwrap();
    let (ids, watermark) = rest(&db, next.cursor.unwrap());
    assert_eq!(ids, named([950]));
    assert_eq!(watermark["rowid_after"], 1_001);
    assert_eq!(watermark["prints"].as_object().unwrap().len(), 1_001);
    let (ids, _) = pass(&db, watermark);
    assert!(ids.is_empty(), "nothing changed");
}

/// A database never read before: its newest sessions come first, and
/// each is read once.
#[test]
fn a_first_read_takes_the_newest_sessions_first_and_each_once() {
    let db = sessions(250);
    let (first, _) = page(&db, &Value::Null);
    assert_eq!(first, named(151..=250));
    let (mut ids, watermark) = pass(&db, Value::Null);
    ids.sort();
    let mut all = named(1..=250);
    all.sort();
    assert_eq!(ids, all);
    assert_eq!(watermark["rowid_after"], 250);
}

/// More new sessions than a page reads, and more again while those are
/// read: every page takes the newest there are, none is left out and
/// none is read twice.
#[test]
fn more_new_sessions_than_a_page_reads_come_newest_first_and_each_once() {
    let db = sessions(300);
    let (_, watermark) = pass(&db, Value::Null);
    (301..=550).for_each(|n| begins(&db, n));
    let (ids, first) = page(&db, &watermark);
    assert_eq!(ids, named(451..=550));
    let mut read_so_far = ids;

    (551..=700).for_each(|n| begins(&db, n));
    let (ids, second) = page(&db, &first.cursor.unwrap());
    assert_eq!(ids, named(601..=700));
    read_so_far.extend(ids);

    // One more, and the page has room for those still waiting: the
    // newest of them first, from each stretch that was passed over.
    begins(&db, 701);
    let (ids, third) = page(&db, &second.cursor.unwrap());
    assert_eq!(ids, named((701..=701).chain(551..=600).chain(402..=450)));
    read_so_far.extend(ids);

    let (ids, watermark) = rest(&db, third.cursor.unwrap());
    read_so_far.extend(ids);
    read_so_far.sort();
    let mut all = named(301..=701);
    all.sort();
    assert_eq!(read_so_far, all);
    assert_eq!(watermark["rowid_after"], 701);
    assert_eq!(watermark["prints"].as_object().unwrap().len(), 701);
    let (ids, _) = pass(&db, watermark);
    assert!(ids.is_empty(), "nothing changed");
}

/// Sessions removed from the end of the table leave the map of counts,
/// as those removed from its middle do.
#[test]
fn sessions_removed_from_the_end_leave_the_map_of_counts() {
    let db = sessions(250);
    let (_, watermark) = pass(&db, Value::Null);
    db.execute_batch(
        "DELETE FROM session_model_usage WHERE session_id IN ('s249','s250');
         DELETE FROM sessions WHERE id IN ('s249','s250');",
    )
    .unwrap();
    let (ids, watermark) = pass(&db, watermark);
    assert!(ids.is_empty());
    let prints = watermark["prints"].as_object().unwrap();
    assert_eq!(prints.len(), 248);
    assert!(!prints.contains_key("250"));
}

/// The pass over the old sessions reads one that was active since the
/// last pass ended, though its counts are as they were: here it ended.
/// Its time is the only change (ticket 54's verifier, mutation v6).
#[test]
fn an_old_session_active_since_the_last_pass_is_read_with_counts_unchanged() {
    let db = sessions(250);
    let (_, watermark) = pass(&db, Value::Null);
    db.execute("UPDATE sessions SET ended_at=1760000900 WHERE id='s20'", [])
        .unwrap();
    let (ids, watermark) = pass(&db, watermark);
    assert_eq!(ids, named([20]));
    let (ids, _) = pass(&db, watermark);
    assert!(ids.is_empty(), "nothing changed");
}

fn far_named(sessions: impl IntoIterator<Item = i64>) -> Vec<String> {
    let far = |n| format!("hermes:{KEY}/default:s{n}");
    sessions.into_iter().map(far).collect()
}

/// Every write to a mirror begins the pass over its old sessions anew,
/// and each pass costs what a pass costs. A mirror's rows are short, so
/// one page looks at ten thousand of them: the pass ends in the page
/// that began it. Hermes's own database is read a hundred a page.
#[test]
fn a_mirrors_pass_over_its_old_sessions_ends_in_the_page_that_began_it() {
    let grow = "UPDATE session_model_usage SET input_tokens=20 WHERE session_id='s400'";
    let db = sessions(1_000);
    mirror(&db, row(1, KEY, "Mac mini", "default"));
    let (ids, watermark) = pass(&db, Value::Null);
    assert_eq!(ids.len(), 1_000);
    // A session begins, and an old one's counts grow.
    begins(&db, 1_001);
    db.execute_batch(grow).unwrap();
    let (ids, only) = page(&db, &watermark);
    assert_eq!(ids, far_named([1_001, 400]));
    assert!(only.cursor.is_none(), "the pass goes on");
    let watermark = only.watermark.unwrap();
    assert_eq!(watermark["rowid_after"], 1_001);
    assert_eq!(watermark["prints"].as_object().unwrap().len(), 1_001);
    let (ids, _) = pass(&db, watermark);
    assert!(ids.is_empty(), "nothing changed");

    // The same sessions in a database that is Hermes's own.
    let db = sessions(1_000);
    let (_, watermark) = pass(&db, Value::Null);
    begins(&db, 1_001);
    db.execute_batch(grow).unwrap();
    let (ids, first) = page(&db, &watermark);
    assert_eq!(ids, named([1_001]));
    let (mut cursor, mut pages) = (first.cursor, 1);
    while let Some(next) = cursor {
        cursor = page(&db, &next).1.cursor;
        pages += 1;
        assert!(pages <= 11, "the pass goes on past {pages} pages");
    }
    assert_eq!(pages, 11);
}

/// A mirror whose every session changed, here by its label, is read in
/// the page that began the pass, each session once (ticket 60). Read a
/// hundred a page, as it was, a new label took a pass for each hundred
/// sessions, and reached the newest of them last.
#[test]
fn a_mirror_whose_every_session_changed_is_read_in_the_page_that_began_the_pass() {
    for count in [1_000, 5_000] {
        let db = sessions(count);
        mirror(&db, row(1, KEY, "Mac mini", "default"));
        let (_, watermark) = pass(&db, Value::Null);
        db.execute("UPDATE hud_mirror SET label='Studio'", [])
            .unwrap();
        let (ids, page) = page(&db, &watermark);
        assert_eq!(ids, far_named(1..=count));
        assert!(page.cursor.is_none(), "the pass goes on: {:?}", page.cursor);
        assert!(page
            .records
            .iter()
            .all(|record| record.machine.as_deref() == Some("Studio")));
        // Nothing is read again by the pass after it.
        let (ids, _) = pass(&db, page.watermark.unwrap());
        assert!(ids.is_empty(), "{} read again", ids.len());
    }
}

/// The same change in Hermes's own database, whose sessions can hold
/// anything, is read a hundred a page as it always was.
#[test]
fn a_database_that_is_no_mirror_is_read_a_hundred_changed_sessions_a_page() {
    let db = sessions(300);
    let (_, watermark) = pass(&db, Value::Null);
    db.execute_batch("UPDATE session_model_usage SET input_tokens=20")
        .unwrap();
    let (mut cursor, mut all, mut pages) = (Some(watermark), Vec::new(), 0);
    while let Some(next) = cursor {
        let (ids, page) = page(&db, &next);
        assert!(ids.len() <= 100, "{} read by one page", ids.len());
        all.extend(ids);
        cursor = page.cursor;
        pages += 1;
        assert!(pages <= 11, "the pass goes on past {pages} pages");
    }
    assert_eq!(all, named(1..=300));
    assert!((3..=4).contains(&pages), "{pages} pages");
}

/// What a page of a mirror reads is bounded all the same: ten thousand
/// changed sessions, and the rest in the page after.
#[test]
fn a_page_reads_ten_thousand_of_a_mirrors_changed_sessions_at_most() {
    let db = sessions(10_050);
    mirror(&db, row(1, KEY, "Mac mini", "default"));
    let (_, watermark) = pass(&db, Value::Null);
    db.execute("UPDATE hud_mirror SET label='Studio'", [])
        .unwrap();
    let (ids, first) = page(&db, &watermark);
    assert_eq!(ids.len(), 10_000);
    let cursor = first.cursor.expect("fifty more changed");
    let (rest, second) = page(&db, &cursor);
    assert_eq!(rest, far_named(10_001..=10_050));
    assert!(second.cursor.is_none());
}

/// And what a page looks at is bounded too: ten thousand old sessions.
#[test]
fn a_page_looks_at_ten_thousand_of_a_mirrors_old_sessions_at_most() {
    let db = sessions(10_050);
    mirror(&db, row(1, KEY, "Mac mini", "default"));
    let (_, watermark) = pass(&db, Value::Null);
    begins(&db, 10_051);
    db.execute_batch("UPDATE session_model_usage SET input_tokens=20 WHERE session_id='s10020'")
        .unwrap();
    let (ids, first) = page(&db, &watermark);
    assert_eq!(ids, far_named([10_051]));
    let cursor = first.cursor.expect("the pass has fifty more to look at");
    assert_eq!(cursor["scan"], 10_000);
    let (ids, second) = page(&db, &cursor);
    assert_eq!(ids, far_named([10_020]));
    assert!(second.cursor.is_none());
}

/// What `hud_live` holds reaches every record of its session as
/// `live_total`, which counts nothing; and a sample that changed reads
/// its session again, and no other.
#[test]
fn a_mirrors_live_total_is_on_the_record_and_a_new_one_reads_the_session_again() {
    use super::super::gateway::{Listed, Sample};
    use super::super::mirror::{list, live};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(format!("profiles/{KEY}-default/state.db"));
    // `s1` as the far database gave it, and `live1` from the live list.
    let db = far_db(&path, Made::Written, "Mac mini", "default");
    let said = Listed {
        title: Some("Listed live".into()),
        started_at: Some(1_760_000_100.0),
        ..Listed::default()
    };
    assert_eq!(list(&db, &[("live1".to_owned(), said)], 1_000).unwrap(), 1);
    let first = read(&db, &path, &Value::Null).unwrap();
    assert_eq!(first.records.len(), 2);
    assert!(first.records.iter().all(|r| r.live_total.is_none()));
    let durable = counted(&first.records[0]);
    let far = |id: &str| format!("hermes:{KEY}/default:{id}");
    let sample = |input, output| Sample {
        input,
        output,
        cache_read: 30,
        cache_write: 5,
    };
    let shown = |page: &Page| {
        let of = |r: &Record| (r.id.clone(), r.live_total, r.usage.total());
        page.records.iter().map(of).collect::<Vec<_>>()
    };

    // A sample: its session is read again, with nothing more counted.
    assert_eq!(
        live(&db, &[("live1".to_owned(), sample(100, 20), 2_000)]).unwrap(),
        1
    );
    let page = read(&db, &path, &first.watermark.unwrap()).unwrap();
    assert_eq!(shown(&page), [(far("live1"), Some(155), 0)]);
    // Nothing changed: nothing is read.
    let watermark = page.watermark.unwrap();
    assert!(read(&db, &path, &watermark).unwrap().records.is_empty());
    // It grew: read again.
    assert_eq!(
        live(&db, &[("live1".to_owned(), sample(200, 40), 3_000)]).unwrap(),
        1
    );
    let page = read(&db, &path, &watermark).unwrap();
    assert_eq!(shown(&page), [(far("live1"), Some(275), 0)]);
    // A session the far database gave has its sample on its records too,
    // and they count what they counted without it.
    assert_eq!(
        live(&db, &[("s1".to_owned(), sample(1, 1), 3_000)]).unwrap(),
        1
    );
    let page = read(&db, &path, &page.watermark.unwrap()).unwrap();
    assert_eq!(shown(&page), [(far("s1"), Some(37), 180)]);
    assert_eq!(counted(&page.records[0]), durable);
    // The whole mirror read anew says the same.
    let whole = read(&db, &path, &Value::Null).unwrap();
    assert_eq!(
        shown(&whole),
        [(far("s1"), Some(37), 180), (far("live1"), Some(275), 0)]
    );
}

/// `hud_live` means something in a mirror alone, and a mirror without
/// the table reads as it did.
#[test]
fn a_database_that_is_no_mirror_gives_no_live_total() {
    let home = tempfile::tempdir().unwrap();
    let live = "CREATE TABLE hud_live (session_id TEXT PRIMARY KEY, input_tokens INTEGER,
        output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, read_ms INTEGER);
        INSERT INTO hud_live VALUES ('s1', 5, 5, 5, 5, 1);";
    // This machine's own database, with a table of that name in it.
    let path = home.path().join("own/state.db");
    let db = state_db(&path, None);
    db.execute_batch(live).unwrap();
    let page = read(&db, &path, &Value::Null).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].live_total, None);
    // A mirror made by hand, without the table and then with it.
    let path = home.path().join(format!("profiles/{KEY}-default/state.db"));
    let db = far_db(&path, Made::ByHand, "Mac mini", "default");
    let page = read(&db, &path, &Value::Null).unwrap();
    assert_eq!(page.records[0].live_total, None);
    db.execute_batch(live).unwrap();
    let page = read(&db, &path, &page.watermark.unwrap()).unwrap();
    assert_eq!(page.records[0].live_total, Some(20));
    // A count past what a number holds is read as the most there is.
    db.execute("UPDATE hud_live SET input_tokens=9223372036854775807", [])
        .unwrap();
    let page = read(&db, &path, &Value::Null).unwrap();
    assert_eq!(page.records[0].live_total, Some(i64::MAX as u64));
}
