//! A Hermes session's state when its gateway is followed live: what the
//! gateway says stands over the hooks' guess for the sessions it lists,
//! and the hooks have every other session, and every session once the
//! gateway says nothing.
use super::*;
use crate::SourceRoot;
use agent_hud_agents::{parse_event, LiveState, LiveStates};
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

/// The channel tells what it read `ago` ms ago.
fn read(core: &AgentHudCore, sessions: &[(&str, LiveState)], ago: i64) {
    let sessions = sessions
        .iter()
        .map(|(id, state)| (id.to_string(), *state))
        .collect();
    let read_ms = now() - ago;
    core.requests()
        .set_native_states("hermes", &LiveStates { sessions, read_ms });
}

/// The channel tells what it has just read.
fn live(core: &AgentHudCore, sessions: &[(&str, LiveState)]) {
    read(core, sessions, 0);
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
    let told = |sessions: Vec<(String, LiveState)>, read_ms| LiveStates { sessions, read_ms };
    requests.set_native_states("hermes", &told(vec![], now()));
    assert_eq!(
        requests.revision(),
        revision,
        "nothing told, nothing to clear"
    );
    let running = vec![(ID.to_owned(), LiveState::Running)];
    let first = now();
    requests.set_native_states("hermes", &told(running.clone(), first));
    let after = requests.revision();
    assert_ne!(after, revision);
    // The same sessions told again from the same reading change nothing.
    requests.set_native_states("hermes", &told(running.clone(), first));
    assert_eq!(requests.revision(), after);
    // A newer reading of the same sessions renews it, and the views are
    // made again (it decides anew against hooks that reported since).
    requests.set_native_states("hermes", &told(running.clone(), first + 1_000));
    let renewed = requests.revision();
    assert_ne!(renewed, after);
    assert_eq!(requests.native_states()["hermes"].read_ms, first + 1_000);
    requests.set_native_states("hermes", &told(vec![], first + 2_000));
    assert_ne!(requests.revision(), renewed);
    assert!(requests.native_states().is_empty());
}

#[test]
fn a_reading_lapses_on_its_own_and_wakes_the_hud() {
    let requests = crate::requests::Requests::default();
    let read_ms = now();
    let sessions = vec![(ID.to_owned(), LiveState::Running)];
    requests.set_native_states("hermes", &LiveStates { sessions, read_ms });
    let lapses = read_ms + crate::requests::LIVE_STATES_MS;
    assert_eq!(requests.next_deadline(), Some(lapses));
    let revision = requests.revision();
    requests.expire(lapses - 1);
    assert_eq!(requests.revision(), revision, "still believed");
    requests.expire(lapses);
    assert_ne!(requests.revision(), revision);
    assert!(requests.native_states().is_empty());
    assert_eq!(requests.next_deadline(), None);
}

/// Verifier, round 1: a channel that read once and then went quiet without
/// a word (a hung poll on a socket that still looks open) must not hold
/// the session over its hooks.
#[test]
fn verifier_stale_live_state_must_return_to_hooks_without_another_publish() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    hook(&mut core, "on_session_end", 5_000);
    assert_eq!(state(&mut core), "idle");
    read(&core, &[(ID, LiveState::Running)], 365 * 24 * 60 * MINUTE);
    core.requests().expire(now());
    assert_eq!(
        state(&mut core),
        "idle",
        "a year-old poll must stop overriding the hooks"
    );
}

/// Verifier, round 1: the same, for a session only the reading kept listed.
#[test]
fn verifier_stale_live_state_must_not_keep_an_ancient_session_visible() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 3 * 24 * 60 * MINUTE);
    assert_eq!(row(&mut core), None);
    read(&core, &[(ID, LiveState::Running)], 365 * 24 * 60 * MINUTE);
    core.requests().expire(now());
    assert_eq!(
        row(&mut core),
        None,
        "an expired poll must release the session from the panel"
    );
}

/// Verifier, round 2: timed by Core's pinned snapshot clock, not by how
/// long the test thread takes.
#[test]
fn a_reading_is_believed_for_thirty_seconds_and_no_longer() {
    let at = now();
    let pin = |ms: i64| crate::snapshot::PINNED_CLOCK.set(Some(ms));
    pin(at);
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 3 * 24 * 60 * MINUTE);
    let sessions = vec![(ID.to_owned(), LiveState::Running)];
    core.requests().set_native_states(
        "hermes",
        &LiveStates {
            sessions,
            read_ms: at,
        },
    );
    pin(at + crate::requests::LIVE_STATES_MS - 1);
    assert_eq!(state(&mut core), "running", "a moment before it lapses");
    pin(at + crate::requests::LIVE_STATES_MS);
    assert_eq!(row(&mut core), None, "lapsed: only the reading listed it");
    crate::snapshot::PINNED_CLOCK.set(None);
}

#[test]
fn a_long_turn_stays_running_while_every_reading_renews_it() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 3 * 24 * 60 * MINUTE);
    hook(&mut core, "pre_tool_call", 20 * MINUTE);
    for _ in 0..3 {
        live(&core, &[(ID, LiveState::Running)]);
        core.requests().expire(now());
        assert_eq!(state(&mut core), "running");
    }
}

#[test]
fn a_turn_end_the_hooks_push_after_the_reading_wins_until_the_next_one() {
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    hook(&mut core, "pre_tool_call", 3 * MINUTE);
    read(&core, &[(ID, LiveState::Running)], 2_000);
    assert_eq!(state(&mut core), "running");
    // The turn ends a second after the reading: the hook is newer news.
    hook(&mut core, "on_session_end", 1_000);
    assert_eq!(state(&mut core), "idle");
    // A reading taken after the hook stands again, whatever it says: here
    // Hermes's list is a moment behind and still says running.
    live(&core, &[(ID, LiveState::Running)]);
    assert_eq!(state(&mut core), "running");
    live(&core, &[(ID, LiveState::Idle)]);
    assert_eq!(state(&mut core), "idle");
}

/// Verifier, round 2: a newer reading of the very same sessions can take a
/// session back from a hook that reported since the last one, so it must
/// wake the HUD, not wait for something else to.
#[test]
fn verifier2_regaining_control_over_a_hook_must_wake_the_display() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let mut core = hermes(root.path(), 10 * MINUTE);
    let wakes = std::sync::Arc::new(AtomicUsize::new(0));
    let counted = wakes.clone();
    core.requests().set_waker(std::sync::Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let woken = || wakes.load(Ordering::SeqCst);

    // A turn end the hooks push after a reading, then the next reading,
    // which still lists the session running.
    read(&core, &[(ID, LiveState::Running)], 3_000);
    hook(&mut core, "on_session_end", 2_000);
    assert_eq!(state(&mut core), "idle");
    let before = woken();
    live(&core, &[(ID, LiveState::Running)]);
    assert!(woken() > before, "the reading took the session back");
    assert_eq!(state(&mut core), "running");

    // A turn start the hooks push after an idle reading, then the next
    // reading, which still lists the session idle.
    live(&core, &[]);
    read(&core, &[(ID, LiveState::Idle)], 3_000);
    hook(&mut core, "pre_llm_call", 2_000);
    assert_eq!(state(&mut core), "running");
    let before = woken();
    read(&core, &[(ID, LiveState::Idle)], 1_000);
    assert!(woken() > before, "the reading took the session back");
    assert_eq!(state(&mut core), "idle");

    // The very same reading told again wakes nothing.
    let sessions = vec![(ID.to_owned(), LiveState::Idle)];
    let read_ms = core.requests().native_states()["hermes"].read_ms;
    let before = woken();
    core.requests()
        .set_native_states("hermes", &LiveStates { sessions, read_ms });
    assert_eq!(woken(), before);
}
