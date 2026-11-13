//! A Hermes session's state when its gateway is followed live: what the
//! gateway says stands over the hooks' guess for the sessions it lists,
//! and the hooks have every other session, and every session once the
//! gateway says nothing.
use super::*;
use crate::SourceRoot;
use agent_hud_agents::{parse_event, LiveState};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;

const SCHEMA: &str = include_str!("../../../agents/tests/fixtures/hermes/macos/state.schema.sql");
const ID: &str = "hermes:s1";
const MINUTE: i64 = 60_000;

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A core reading a Hermes `state.db` with one session, `s1`, that
/// started `ago` ms ago and has not been written to since.
fn hermes(root: &Path, ago: i64) -> AgentHudCore {
    let folder = root.join("hermes");
    fs::create_dir_all(&folder).unwrap();
    let db = Connection::open(folder.join("state.db")).unwrap();
    db.execute_batch(SCHEMA).unwrap();
    let started = (now() - ago) as f64 / 1000.0;
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES('s1','cron','m',?1,?1)",
        [started],
    )
    .unwrap();
    AgentHudCore::new(
        vec![SourceRoot {
            path: folder.join("state.db"),
            provider: "hermes",
        }],
        root.join("hud"),
    )
}

/// A Hermes hook for `s1` that ran `ago` ms ago, as the hook command
/// hands it to core.
fn hook(core: &mut AgentHudCore, event: &str, ago: i64) {
    let payload = json!({"hook_event_name": event, "session_id": "s1", "cwd": "/tmp",
        "extra": {"turn_id": "s1:turn-1", "surface": "cli"}});
    let event = parse_event("hermes", "--completion-hook", &payload, now() - ago)
        .unwrap()
        .unwrap();
    core.ingest_hook_events(vec![event]);
}

fn live(core: &AgentHudCore, sessions: &[(&str, LiveState)]) {
    let sessions: Vec<(String, LiveState)> = sessions
        .iter()
        .map(|(id, state)| (id.to_string(), *state))
        .collect();
    core.requests()
        .set_native_states("hermes", &sessions, now());
}

/// The session's row in a fresh snapshot, or None where it is not listed.
fn row(core: &mut AgentHudCore) -> Option<Value> {
    let mut snapshot = core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        if core.reads.pending.is_empty() {
            break;
        }
        snapshot = core.request("snapshot", json!({})).unwrap();
    }
    let sessions = snapshot["sessions"].as_array()?;
    sessions.iter().find(|row| row["id"] == ID).cloned()
}

fn state(core: &mut AgentHudCore) -> String {
    let row = row(core).expect("the session is listed");
    row["state"].as_str().unwrap().to_owned()
}

#[test]
fn a_long_tool_call_runs_for_as_long_as_the_gateway_says_it_does() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    // The turn's last hook came three minutes ago: past the hooks' two
    // minutes, so on their own they guess it is over.
    hook(&mut core, "pre_tool_call", 3 * MINUTE);
    assert_eq!(state(&mut core), "idle");
    live(&core, &[(ID, LiveState::Running)]);
    assert_eq!(state(&mut core), "running");
    // The gateway says nothing more (stopped, or the socket dropped): the
    // hooks' guess is back.
    live(&core, &[]);
    assert_eq!(state(&mut core), "idle");
}

#[test]
fn a_question_waits_however_long_it_takes_to_answer() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 3 * 24 * 60 * MINUTE);
    assert_eq!(row(&mut core), None, "three days quiet, it is not listed");
    // A session that waits is listed, however long ago it last wrote.
    live(&core, &[(ID, LiveState::Waiting)]);
    assert_eq!(state(&mut core), "waiting");
    // Its hooks said it began waiting 45 minutes ago, which they no
    // longer believe on their own.
    hook(&mut core, "pre_approval_request", 45 * MINUTE);
    assert_eq!(state(&mut core), "waiting");
    live(&core, &[]);
    assert_eq!(state(&mut core), "idle");
    // Answered: the turn runs on.
    live(&core, &[(ID, LiveState::Running)]);
    assert_eq!(state(&mut core), "running");
}

#[test]
fn a_session_with_a_card_waits_whatever_the_gateway_lists() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    hook(&mut core, "pre_tool_call", 10_000);
    live(&core, &[(ID, LiveState::Running)]);
    core.requests()
        .set_native_waiting("hermes", &[ID.to_owned()], now());
    assert_eq!(state(&mut core), "waiting");
}

#[test]
fn a_session_the_gateway_does_not_list_follows_its_hooks() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    live(&core, &[("hermes:other", LiveState::Running)]);
    hook(&mut core, "pre_tool_call", 10_000);
    assert_eq!(state(&mut core), "running");
    hook(&mut core, "on_session_end", 5_000);
    assert_eq!(state(&mut core), "idle");
}

#[test]
fn the_gateway_says_when_a_turn_runs_and_when_it_is_over() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    // A turn the hooks saw end, and a new one the gateway runs (a
    // scheduled job, or the desktop app with no hook yet).
    hook(&mut core, "on_session_end", 30_000);
    let ended = row(&mut core).unwrap();
    assert_eq!(ended["state"], "idle");
    assert!(ended["liveFacts"]["endedAt"].is_string());
    live(&core, &[(ID, LiveState::Running)]);
    let running = row(&mut core).unwrap();
    assert_eq!(running["state"], "running");
    assert_eq!(running["liveFacts"]["endedAt"], Value::Null);
    assert_eq!(running["liveFacts"]["outcome"], Value::Null);
    // The hooks say a turn runs; the gateway, read since, says it is over.
    hook(&mut core, "pre_tool_call", 5_000);
    live(&core, &[(ID, LiveState::Idle)]);
    assert_eq!(state(&mut core), "idle");
}

#[test]
fn telling_the_same_states_again_changes_nothing() {
    let requests = crate::requests::Requests::default();
    let revision = requests.revision();
    requests.set_native_states("hermes", &[], now());
    assert_eq!(
        requests.revision(),
        revision,
        "nothing told, nothing to clear"
    );
    let told = [(ID.to_owned(), LiveState::Running)];
    requests.set_native_states("hermes", &told, now());
    let after = requests.revision();
    assert_ne!(after, revision);
    // A channel tells after everything it does; only a change counts.
    requests.set_native_states("hermes", &told, now());
    assert_eq!(requests.revision(), after);
    requests.set_native_states("hermes", &[], now());
    assert_ne!(requests.revision(), after);
    assert!(requests.native_states().is_empty());
}
