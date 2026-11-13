//! Where a Hermes home's sessions are saved in the ledger, by the home
//! alone: the second review of the machine identity
//! (`evidence/22/verifier-2-files/persistence.rs`) showed that the id the
//! reader builds from a `state.db`'s folder is saved, so it must not move
//! to another namespace, and that a root in a folder no profile is named
//! as must still be read.
use super::*;

/// The Hermes session ids the ledger holds after one whole collection of
/// the `state.db` in `home`, and how many saved heads carry the first.
fn saved(home: &Path, data: &Path) -> (Vec<String>, i64) {
    fs::create_dir_all(home).unwrap(); // plain directory
    let db = rusqlite::Connection::open(home.join("state.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions(id TEXT,source TEXT,model TEXT,started_at REAL,input_tokens INTEGER DEFAULT 0,output_tokens INTEGER DEFAULT 0,profile_name TEXT);
         INSERT INTO sessions VALUES('s1','cli','loopback',1760000000,10,2,'default');",
    )
    .unwrap();
    drop(db);
    let root = SourceRoot {
        path: home.to_owned(),
        provider: "hermes",
    };
    let mut core = AgentHudCore::new(vec![root], data.to_owned());
    core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        core.request("snapshot", json!({})).unwrap();
    }
    drop(core);
    let ledger = rusqlite::Connection::open_with_flags(
        data.join("ingestion.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut rows = ledger
        .prepare("SELECT session_id FROM session_mapping WHERE source='hermes'")
        .unwrap();
    let ids: Vec<String> = rows
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let first = ids.first().cloned().unwrap_or_default();
    let heads = ledger
        .query_row(
            "SELECT COUNT(*) FROM ingestion_logs WHERE json_extract(head,'$.id')=?1",
            [first],
            |row| row.get(0),
        )
        .unwrap();
    (ids, heads)
}

/// The verifier's case: a home at `…/profiles/work`, its one row labelled
/// `default`. Its id was saved as the named profile's, and still is.
#[test]
fn custom_root_path_guess_reaches_the_durable_ledger() {
    let temp = tempfile::tempdir().unwrap();
    let (ids, heads) = saved(&temp.path().join("profiles/work"), &temp.path().join("app"));
    assert_eq!(ids, ["hermes:work:s1"]);
    assert!(heads > 0);
}

/// A root in a folder no profile is named as is collected, as its default
/// profile, and saved under an id nothing in its path is part of.
#[test]
fn a_custom_root_no_profile_is_named_as_reaches_the_durable_ledger() {
    for folder in ["profiles/Work", "profiles/work.name", "profiles/.hermes"] {
        let temp = tempfile::tempdir().unwrap();
        let (ids, heads) = saved(&temp.path().join(folder), &temp.path().join("app"));
        assert_eq!(ids, ["hermes:s1"], "{folder}");
        assert!(heads > 0, "{folder}");
    }
}
