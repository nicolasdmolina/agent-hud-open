//! A native card's session sounds once while its row changes source (ticket
//! 41, after its first verifier). Codex Desktop makes one question card per
//! question of one `request_user_input_async` call, with no `callId`
//! (`platform/src/clients/codex/desktop/pending.rs`); the cards' session gets
//! a row of its own until its rollout is read, and the rollout then shows the
//! call once. That one sighting cannot be paired with two unnamed cards, and
//! must not sound again for a question that already did.
use super::*;

struct Clock;
impl Clock {
    fn at(now: i64) -> Self {
        crate::snapshot::PINNED_CLOCK.set(Some(now));
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        crate::snapshot::PINNED_CLOCK.set(None);
    }
}

const THREAD: &str = "00000000-0000-4000-8000-000000000041";

fn needs_you(snapshot: &Value) -> Vec<Value> {
    snapshot["soundAlerts"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|alert| alert["kind"] == "needs-you")
        .map(|alert| alert["id"].clone())
        .collect()
}

fn row(snapshot: &Value) -> Vec<Value> {
    snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == THREAD)
        .cloned()
        .collect()
}

/// The card Codex Desktop's parser makes for question `index`: kind
/// `question`, no `callId`.
fn question(core: &AgentHudCore, index: usize, now: i64) -> requests::Ticket {
    let view = json!({"source":"codex","vendor":"Codex","sessionId":THREAD,"title":"Synthetic","summary":"Synthetic","toolName":"request_user_input_async","kind":"question","input":"","detail":null,"cwd":"/synthetic","createdAt":crate::iso(now),"allowsAlways":false,"answersQuestion":true,"answersPlan":false,"plan":null,"suggestions":[],
        "questions":[{"id":format!("q{index}"),"answerKey":format!("q{index}"),"question":format!("Synthetic question {index}"),"options":[{"label":"First"},{"label":"Second"}],"multiSelect":false}]});
    core.requests()
        .register_native(format!("codex-desktop-question:{index}"), view, now)
        .unwrap()
}

/// A Core reading `folder` as Codex's sessions, with sound on.
fn codex_core(temp: &Path) -> (AgentHudCore, PathBuf) {
    let folder = temp.join(".codex/sessions");
    fs::create_dir_all(&folder).unwrap();
    let mut core = AgentHudCore::new(vec![], temp.join("app"));
    core.provider_roots = vec![(folder.clone(), "codex")];
    core.apply_setting("sound", &json!(true)).unwrap();
    core.request("snapshot", json!({})).unwrap();
    (core, folder)
}

/// The rollout of the call that asked: one `request_user_input_async`.
fn write_rollout(folder: &Path, now: i64) {
    let lines = [
        json!({"timestamp":crate::iso(now-3000),"type":"session_meta","payload":{"id":THREAD,"timestamp":crate::iso(now-3000),"source":"vscode","originator":"Codex Desktop","cwd":"/synthetic"}}),
        json!({"timestamp":crate::iso(now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-41"}}),
        json!({"timestamp":crate::iso(now-1000),"type":"response_item","payload":{"type":"function_call","call_id":"call-41","name":"request_user_input_async","arguments":"{}"}}),
    ];
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    fs::write(folder.join(format!("rollout-{THREAD}.jsonl")), text).unwrap();
}

/// `count` question cards; unless `together`, a snapshot shows them before
/// the rollout is read `delay_ms` later. Returns the alerts before and after.
fn handover(count: usize, together: bool, delay_ms: i64) -> (Vec<Value>, Vec<Value>) {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let (mut core, folder) = codex_core(temp.path());
    let _tickets: Vec<_> = (0..count).map(|i| question(&core, i, now)).collect();
    let mut before = Vec::new();
    if !together {
        let first = core.request("request_snapshot", json!({})).unwrap();
        assert_eq!(row(&first)[0]["evidence"], "native-card");
        before = needs_you(&first);
    }
    crate::snapshot::PINNED_CLOCK.set(Some(now + delay_ms));
    write_rollout(&folder, now);
    let after = core.request("refresh", json!({})).unwrap();
    let rows = row(&after);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["evidence"], Value::Null, "the rollout's own row");
    assert_eq!(rows[0]["state"], "waiting");
    let again = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(
        needs_you(&again),
        needs_you(&after),
        "a later look adds none"
    );
    (before, needs_you(&after))
}

#[test]
fn one_question_card_sounds_once_when_its_rollout_row_takes_over() {
    let (before, after) = handover(1, false, 0);
    assert_eq!(before.len(), 1);
    assert_eq!(after, before);
}

#[test]
fn a_multi_question_call_sounds_once_when_its_rollout_row_takes_over() {
    let (before, after) = handover(2, false, 0);
    assert_eq!(before.len(), 1, "two cards at one look make one sound");
    assert_eq!(
        after, before,
        "the rollout's one sighting of the call is no new question"
    );
}

#[test]
fn a_multi_question_call_sounds_once_when_its_rollout_comes_five_minutes_later() {
    let (before, after) = handover(2, false, 300_000);
    assert_eq!(before.len(), 1);
    assert_eq!(after, before);
}

#[test]
fn a_multi_question_call_and_its_rollout_seen_together_sound_once() {
    let (_, after) = handover(2, true, 0);
    assert_eq!(after.len(), 1);
}

/// The takeover silences only what its cards already told: a card new at
/// that very look still sounds.
#[test]
fn a_new_card_at_the_takeover_still_sounds() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let (mut core, folder) = codex_core(temp.path());
    let _first = question(&core, 0, now);
    let before = needs_you(&core.request("request_snapshot", json!({})).unwrap());
    assert_eq!(before.len(), 1);
    let _second = question(&core, 1, now + 1);
    write_rollout(&folder, now);
    let after = needs_you(&core.request("refresh", json!({})).unwrap());
    assert_eq!(after.len(), 2, "the second card is a new question");
}
