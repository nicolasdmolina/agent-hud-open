//! Files a native channel says it wrote (`hooks::Changed`) are read at
//! the request turn its wake asks for, whatever the file watcher saw. And
//! what it says in a hook's place (`hooks::Told`) is core's at that turn.
use super::*;
use agent_hud_core::SourceRoot;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What the file watcher's (`notify`) threads are named by: it calls its
/// handlers on one, `notify-rs fsevents loop` on macOS.
const FILE_WATCHER: &str = "notify-rs";

/// One reply of a Claude Code session, which counts 30 tokens.
fn reply(session: &str) -> String {
    let timestamp = (chrono::Utc::now() - chrono::Duration::seconds(60)).to_rfc3339();
    let line = json!({"type": "assistant", "sessionId": session, "timestamp": timestamp,
        "message": {"id": "m1", "model": "claude-opus-4-5", "role": "assistant",
            "content": [{"type": "text", "text": "Done."}],
            "usage": {"input_tokens": 10, "output_tokens": 20,
                "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}});
    format!("{line}\n")
}

#[test]
fn a_file_a_native_channel_says_it_wrote_is_read_at_the_turn_its_wake_asks_for() {
    let temp = tempfile::tempdir().unwrap();
    let logs = temp.path().join(".claude/projects");
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(&logs).unwrap(); // plain directory
    fs::create_dir_all(&elsewhere).unwrap(); // plain directory
    let root = |path: &Path| SourceRoot {
        path: path.to_owned(),
        provider: "claude",
    };
    let mut runtime =
        Runtime::with_sources(vec![root(&logs)], temp.path().join("app-data")).unwrap();
    // The file watcher's events are withheld: it watches another folder,
    // so it runs and has nothing to say of the logs.
    runtime.changes = FileChanges::new(&[root(&elsewhere)]);
    let woken = Arc::new(AtomicUsize::new(0));
    let counted = woken.clone();
    runtime.set_request_waker(Arc::new(move || {
        // The hook inbox's watcher wakes this collector too, from the file
        // watcher's own thread, when the events of the inbox folder it
        // just made reach it late (a loaded machine, ticket 56). That is
        // not a request's wake, nor a written file's: not counted.
        let thread = std::thread::current();
        if !thread
            .name()
            .is_some_and(|name| name.starts_with(FILE_WATCHER))
        {
            counted.fetch_add(1, Ordering::AcqRel);
        }
    }));
    let total = |snapshot: Value| snapshot["totals"]["total"].clone();
    let turn =
        |runtime: &mut Runtime, method: &str| total(runtime.request(method, json!({})).unwrap());
    assert_eq!(turn(&mut runtime, "snapshot"), 0);

    // A log core has never read, written with no event: neither a
    // request turn nor a pass finds it.
    let path = logs.join("31111111-2222-3333-4444-555555555555.jsonl");
    fs::write(&path, reply("31111111-2222-3333-4444-555555555555")).unwrap();
    assert_eq!(turn(&mut runtime, "request_snapshot"), 0);
    assert_eq!(turn(&mut runtime, "snapshot"), 0);
    assert_eq!(woken.load(Ordering::Acquire), 0);

    // A native channel says it wrote the file: the collector is woken,
    // and the request turn that follows reads it.
    let changed = runtime.hooks_handle().lock().unwrap().files_changed();
    changed(std::slice::from_ref(&path));
    assert_eq!(woken.load(Ordering::Acquire), 1);
    assert_eq!(turn(&mut runtime, "request_snapshot"), 30);
    // Told once, read once: the next request turn is a plain one.
    let hooks = runtime.hooks_handle();
    assert!(hooks.lock().unwrap().drain_file_changes().is_empty());
    assert_eq!(turn(&mut runtime, "request_snapshot"), 30);

    // Nothing told wakes nobody.
    changed(&[]);
    assert_eq!(woken.load(Ordering::Acquire), 1);
}

/// Ticket 54: a file core could not read is left be for 30 s and
/// longer, whatever the file watcher says of it. Its writer saying it
/// wrote it is the reason to read it again, at that turn.
#[test]
fn a_file_a_native_channel_says_it_wrote_is_read_though_its_last_read_failed() {
    let temp = tempfile::tempdir().unwrap();
    let mirrors = temp.path().join("hermes-far");
    let elsewhere = temp.path().join("elsewhere");
    let path = mirrors.join("profiles/made-up/state.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    fs::create_dir_all(&elsewhere).unwrap(); // plain directory
    let root = |path: &Path, provider: &'static str| SourceRoot {
        path: path.to_owned(),
        provider,
    };
    // A database with no table in it yet, as a writer's file is between
    // being made and its first commit.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    assert!(fs::metadata(&path).unwrap().len() > 0);
    let mut runtime =
        Runtime::with_sources(vec![root(&mirrors, "hermes")], temp.path().join("app-data"))
            .unwrap();
    runtime.changes = FileChanges::new(&[root(&elsewhere, "claude")]);
    let total = |snapshot: Value| snapshot["totals"]["total"].clone();
    let turn =
        |runtime: &mut Runtime, method: &str| total(runtime.request(method, json!({})).unwrap());
    // The first pass walks the folder, finds the file and cannot read it.
    assert_eq!(turn(&mut runtime, "refresh"), 0);

    // The first commit: Hermes's tables, and one session of 30 tokens.
    let started = chrono::Utc::now().timestamp() - 60;
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT, model TEXT, started_at REAL);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL,
            input_tokens INTEGER, output_tokens INTEGER, first_seen REAL, last_seen REAL);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES ('s1', 'telegram', 'claude-test', ?1)",
        [started],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage VALUES ('s1', 'claude-test', 10, 20, ?1, ?1)",
        [started],
    )
    .unwrap();
    // A pass that hears of nothing leaves the file be: it pauses.
    assert_eq!(turn(&mut runtime, "snapshot"), 0);

    // Its writer says it wrote it: the turn that follows reads it.
    let changed = runtime.hooks_handle().lock().unwrap().files_changed();
    changed(std::slice::from_ref(&path));
    assert_eq!(turn(&mut runtime, "request_snapshot"), 30);
}

#[test]
fn a_turn_a_native_channel_told_is_cores_at_the_turn_its_wake_asks_for() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::with_sources(vec![], temp.path().join("app-data")).unwrap();
    let woken = Arc::new(AtomicUsize::new(0));
    let counted = woken.clone();
    runtime.set_request_waker(Arc::new(move || {
        // Not the hook inbox watcher's wakes, from the file watcher's own
        // thread, as in the test above (tickets 56 and 59).
        let thread = std::thread::current();
        if !thread
            .name()
            .is_some_and(|name| name.starts_with(FILE_WATCHER))
        {
            counted.fetch_add(1, Ordering::AcqRel);
        }
    }));
    let sound = json!({"key": "sound", "value": true});
    runtime.request("set_setting", sound).unwrap();
    let first = runtime.request("snapshot", json!({})).unwrap();
    assert_eq!(first["soundAlerts"]["items"], json!([]));

    // A channel that follows a client on another machine read a turn to
    // start and to end, and says each as that client's hook would.
    let session = "hermes:0123456789abcdef/default:far-1";
    let seen = chrono::Utc::now().timestamp_millis();
    // As a hook event is written (this crate names no client's types).
    let fact = |kind: &str, data: Value| json!({"source": "hermes", "sessionId": session, "atMs": seen, "kind": kind, "data": data});
    let told = runtime.hooks_handle().lock().unwrap().told();
    let started = json!({"turnId": "epoch-1:2", "state": "running", "startedAtMs": null,
        "observedAtMs": seen, "workspace": null, "outcome": null,
        "navigationTarget": null, "machine": "Mac mini"});
    let ended = json!({"turnId": "epoch-1:2", "workspace": null, "model": null,
        "machine": "Mac mini"});
    told(serde_json::from_value(fact("session-observation", started)).unwrap());
    told(serde_json::from_value(fact("completion", ended)).unwrap());
    assert_eq!(woken.load(Ordering::Acquire), 2);

    // The request turn that wake asks for hands both to core: the
    // session has a row that names its machine, and its turn's end is
    // an alert, once.
    for _ in 0..3 {
        let snapshot = runtime.request("request_snapshot", json!({})).unwrap();
        let rows = snapshot["sessions"].as_array().unwrap();
        let row = rows.iter().find(|row| row["id"] == session).expect("a row");
        assert_eq!(row["machine"], "Mac mini");
        assert_eq!(row["liveFacts"]["outcome"], "completed");
        let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0]["kind"], "finished");
        assert_eq!(alerts[0]["sessionId"], session);
        let id = format!("finished:hermes:{session}:epoch-1:2");
        assert_eq!(alerts[0]["id"], id.as_str());
    }
}
