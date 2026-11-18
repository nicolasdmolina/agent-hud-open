//! A sample of what a far session has used so far (`live.rs`): where it
//! is kept, which samples are dropped, and that it touches nothing the
//! ledger reads.
use super::{dump, mirror, said};
use crate::hermes::gateway::Sample;
use crate::hermes::mirror::{list, live};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn sample(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Sample {
    Sample {
        input,
        output,
        cache_read,
        cache_write,
    }
}

/// `sessions`, each with a row from the live list.
fn listed(db: &Connection, sessions: &[&str]) {
    let rows: Vec<_> = (sessions.iter())
        .map(|id| {
            (
                (*id).to_owned(),
                said("Listed", "claude-test", "telegram", 1.0),
            )
        })
        .collect();
    assert_eq!(list(db, &rows, 1_000).unwrap(), sessions.len());
}

/// What `hud_live` holds: session, the four counts, when it was read.
fn kept(db: &Connection) -> Vec<(String, [i64; 4], i64)> {
    let sql = "SELECT session_id, input_tokens, output_tokens, cache_read_tokens,
        cache_write_tokens, read_ms FROM hud_live ORDER BY session_id";
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement.query_map([], |row| {
        let counts = [row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?];
        Ok((row.get(0)?, counts, row.get(5)?))
    });
    rows.unwrap().map(Result::unwrap).collect()
}

/// Everything but `hud_live`: what a sample may never change.
fn durable(db: &Connection) -> String {
    let all = dump(db);
    let rest = all.lines().filter(|line| !line.starts_with("hud_live "));
    rest.collect::<Vec<_>>().join("\n")
}

#[test]
fn a_sample_is_kept_with_its_four_counts_and_when_it_was_read() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    listed(&db, &["s1", "s2"]);
    let before = durable(&db);
    let samples = [
        ("s1".to_owned(), sample(100, 20, 30, 5), 2_000),
        ("s2".to_owned(), sample(7, 0, 0, 0), 2_500),
    ];
    assert_eq!(live(&db, &samples).unwrap(), 2);
    assert_eq!(
        kept(&db),
        [
            ("s1".to_owned(), [100, 20, 30, 5], 2_000),
            ("s2".to_owned(), [7, 0, 0, 0], 2_500),
        ]
    );
    // A sample that grew takes the place of the one before it.
    assert_eq!(
        live(&db, &[("s1".to_owned(), sample(150, 30, 60, 5), 3_000)]).unwrap(),
        1
    );
    assert_eq!(kept(&db)[0], ("s1".to_owned(), [150, 30, 60, 5], 3_000));
    // No row the ledger reads, and no other table of the mirror's, was
    // touched.
    assert_eq!(durable(&db), before);
}

#[test]
fn a_zero_a_lower_and_an_older_sample_are_dropped() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    listed(&db, &["s1", "s2"]);
    // Zeros are what Hermes answers where no agent is loaded: no count.
    assert_eq!(
        live(&db, &[("s2".to_owned(), sample(0, 0, 0, 0), 2_000)]).unwrap(),
        0
    );
    assert_eq!(kept(&db), []);
    assert_eq!(
        live(&db, &[("s1".to_owned(), sample(100, 20, 0, 0), 2_000)]).unwrap(),
        1
    );
    let stored = kept(&db);
    for (dropped, why) in [
        (("s1".to_owned(), sample(0, 0, 0, 0), 3_000), "zero"),
        (("s1".to_owned(), sample(90, 20, 0, 0), 3_000), "lower"),
        (
            ("s1".to_owned(), sample(10, 10, 10, 10), 3_000),
            "lower, of other kinds",
        ),
        (("s1".to_owned(), sample(500, 20, 0, 0), 1_999), "older"),
        (
            ("s1".to_owned(), sample(500, 20, 0, 0), 2_000),
            "read at the same time",
        ),
    ] {
        assert_eq!(live(&db, &[dropped]).unwrap(), 0, "{why}");
        assert_eq!(kept(&db), stored, "{why}");
    }
    // One that is neither is kept, among ones that are dropped.
    let mixed = [
        ("s1".to_owned(), sample(0, 0, 0, 0), 4_000),
        ("s1".to_owned(), sample(130, 20, 0, 0), 4_000),
        ("s2".to_owned(), sample(0, 0, 0, 0), 4_000),
    ];
    assert_eq!(live(&db, &mixed).unwrap(), 1);
    assert_eq!(kept(&db), [("s1".to_owned(), [130, 20, 0, 0], 4_000)]);
}

#[test]
fn the_same_sample_written_twice_changes_nothing_the_second_time() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    listed(&db, &["s1"]);
    let one = [("s1".to_owned(), sample(100, 20, 30, 5), 2_000)];
    assert_eq!(live(&db, &one).unwrap(), 1);
    let after = dump(&db);
    assert_eq!(live(&db, &one).unwrap(), 0);
    // Nor the same counts read later: nothing shown would change.
    assert_eq!(
        live(&db, &[("s1".to_owned(), sample(100, 20, 30, 5), 9_000)]).unwrap(),
        0
    );
    assert_eq!(dump(&db), after);
}

#[test]
fn a_sample_of_a_session_the_mirror_has_no_row_for_is_not_kept() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    listed(&db, &["s1"]);
    let before = dump(&db);
    assert_eq!(
        live(
            &db,
            &[("stranger".to_owned(), sample(100, 20, 0, 0), 2_000)]
        )
        .unwrap(),
        0
    );
    assert_eq!(dump(&db), before);
}

#[test]
fn a_count_too_large_to_keep_is_kept_as_the_largest_there_is() {
    let home = tempfile::tempdir().unwrap();
    let (db, _) = mirror(&home, "Mac mini");
    listed(&db, &["s1"]);
    assert_eq!(
        live(&db, &[("s1".to_owned(), sample(u64::MAX, 1, 0, 0), 2_000)]).unwrap(),
        1
    );
    assert_eq!(kept(&db), [("s1".to_owned(), [i64::MAX, 1, 0, 0], 2_000)]);
}

/// Every Rust and Python source file of the crates that is no test.
fn sources(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(
                name.as_str(),
                "tests" | "target" | "fixtures" | "node_modules"
            ) {
                sources(&path, found);
            }
        } else if (name.ends_with(".rs") || name.ends_with(".py"))
            && !name.ends_with("tests.rs")
            && !name.starts_with("tests_")
        {
            found.push(path);
        }
    }
}

/// The tables a statement of `text` writes: the name after each
/// `INSERT INTO`, `REPLACE INTO` and `DELETE FROM`, and between `UPDATE`
/// and `SET`.
fn written(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut tables = Vec::new();
    for (at, word) in words.iter().enumerate() {
        let before = at.checked_sub(1).map(|at| words[at].as_str());
        let then = words.get(at + 2).map(String::as_str);
        let writes = match word.as_str() {
            "into" => matches!(before, Some("insert" | "replace")),
            "from" => before == Some("delete"),
            // `UPDATE <table> SET`; not the `DO UPDATE SET` of an upsert.
            "update" => then == Some("set"),
            _ => false,
        };
        if let (true, Some(table)) = (writes, words.get(at + 1)) {
            tables.push(table.clone());
        }
    }
    tables
}

/// A history batch never writes `hud_live`, and a live sample never
/// writes a durable table: `live.rs` is the one place in the crates that
/// writes `hud_live`, and it writes nothing else.
#[test]
fn only_the_live_write_writes_hud_live_and_it_writes_nothing_else() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    sources(crates, &mut files);
    assert!(files.len() > 400, "{} files", files.len());
    let own = Path::new("agents/src/hermes/mirror/live.rs");
    let mut writers = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let tables = written(&text);
        if file.ends_with(own) {
            assert_eq!(tables, ["hud_live"], "what the live write writes");
        } else if tables.iter().any(|table| table == "hud_live") {
            writers.push(file.clone());
        }
    }
    assert_eq!(writers, [] as [PathBuf; 0], "writes hud_live");
    assert!(files.iter().any(|file| file.ends_with(own)));
    // The check sees a write where there is one.
    assert_eq!(
        written("insert or replace INTO hud_live VALUES (1); DELETE FROM sessions; UPDATE\n hud_rows SET x=1"),
        ["hud_live", "sessions", "hud_rows"]
    );
}
