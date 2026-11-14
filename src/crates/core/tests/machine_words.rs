//! Core says each machine's state in the app's language: the words a
//! machines list shows, as `stateText`. The language is one setting for the
//! whole process, so this is the only test in its binary: nothing else can
//! switch it between the setting and the snapshot.
//!
//! `tests/fixtures/machine-words.json` holds what a channel tells and what
//! the snapshot carries in Chinese and in English. The app's
//! `test/machines-list.test.ts` draws the Chinese list from the same file.
use agent_hud_agents::{MachineState, MachineStatus, Machines};
use agent_hud_core::AgentHudCore;
use serde_json::{json, Value};

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/machine-words.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The machines as the fixture says a channel tells them.
fn told(fixture: &Value) -> Machines {
    let list = fixture["told"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| MachineStatus {
            id: item["id"].as_str().unwrap().into(),
            label: item["label"].as_str().unwrap().into(),
            state: MachineState::from_word(item["state"].as_str().unwrap()).unwrap(),
            note: item["note"].as_str().map(str::to_owned),
            contact_ms: item["contactMs"].as_i64(),
            history_checked_ms: item["historyMs"].as_i64(),
            here: item["here"].as_bool().unwrap_or(false),
        });
    Machines {
        list: list.collect(),
        list_note: fixture["listNote"].as_str().map(str::to_owned),
    }
}

#[test]
fn every_state_is_worded_in_the_language_the_setting_names() {
    let fixture = fixture();
    let source = fixture["source"].as_str().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut core = AgentHudCore::new(Vec::new(), temp.path().join("data"));
    core.requests().set_machines(source, &told(&fixture));
    // Chinese, then English, then Chinese again: the words follow the
    // setting each time, never the language they were first made in.
    for language in ["zh-CN", "en", "zh-CN"] {
        let snapshot = core
            .request("set_setting", json!({"key": "language", "value": language}))
            .unwrap();
        assert_eq!(
            snapshot["machines"][source], fixture["snapshot"][language],
            "machines in {language}"
        );
        assert_eq!(snapshot["machinesNote"][source], fixture["listNote"]);
    }
}
