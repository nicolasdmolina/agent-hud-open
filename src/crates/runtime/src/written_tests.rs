//! Files a native channel says it wrote (`hooks::Changed`) are read at
//! the request turn its wake asks for, whatever the file watcher saw.
use super::*;
use agent_hud_core::SourceRoot;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        counted.fetch_add(1, Ordering::AcqRel);
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
