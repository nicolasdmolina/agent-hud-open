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
    write_rollout_call(folder, now, "call-41", now - 1000);
}

/// A rollout whose one call is `call`, a `request_user_input_async` at `at`.
fn write_rollout_call(folder: &Path, now: i64, call: &str, at: i64) {
    write_rollout_calls(folder, now, &[call], at);
}

/// A rollout whose `calls` are each a `request_user_input_async` at `at`.
fn write_rollout_calls(folder: &Path, now: i64, calls: &[&str], at: i64) {
    let mut lines = vec![
        json!({"timestamp":crate::iso(now-3000),"type":"session_meta","payload":{"id":THREAD,"timestamp":crate::iso(now-3000),"source":"vscode","originator":"Codex Desktop","cwd":"/synthetic"}}),
        json!({"timestamp":crate::iso(now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-41"}}),
    ];
    for call in calls {
        lines.push(json!({"timestamp":crate::iso(at),"type":"response_item","payload":{"type":"function_call","call_id":call,"name":"request_user_input_async","arguments":"{}"}}));
    }
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

/// The permission card Codex Desktop's parser makes for an app's tool call
/// (an MCP elicitation): kind `permission`, no `callId`.
fn permission(core: &AgentHudCore, now: i64) -> (String, requests::Ticket) {
    let id = "codex-desktop-permission:old".to_owned();
    let view = json!({"source":"codex","vendor":"Codex","sessionId":THREAD,"title":"Synthetic app","summary":"Synthetic app","toolName":"computer-use","kind":"permission","input":"","detail":null,"cwd":"/synthetic","createdAt":crate::iso(now),"allowsAlways":true,"questions":[],"answersQuestion":false,"answersPlan":false,"plan":null,"suggestions":[]});
    let ticket = core
        .requests()
        .register_native(id.clone(), view, now)
        .unwrap();
    (id, ticket)
}

/// An older permission card waits and has sounded. The rollout's row, when it
/// takes over, shows a different request: a question of a call of its own.
/// The second verifier's probes (`verifier-2/silence-probes.rs`).
fn question_beside_an_older_permission(at_transcript: bool) {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let (mut core, folder) = codex_core(temp.path());
    let (old, _permission) = permission(&core, now);
    let first = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(needs_you(&first).len(), 1);
    assert_eq!(row(&first)[0]["evidence"], "native-card");
    crate::snapshot::PINNED_CLOCK.set(Some(now + 1000));
    write_rollout_call(&folder, now, "separate-new-question", now + 1000);
    let takeover = core.request("refresh", json!({})).unwrap();
    let rows = row(&takeover);
    assert_eq!(rows[0]["evidence"], Value::Null);
    assert_eq!(
        rows[0]["liveFacts"]["requests"].as_array().unwrap().len(),
        2
    );
    if at_transcript {
        assert_eq!(
            needs_you(&takeover).len(),
            2,
            "a permission card cannot stand for a question"
        );
    }
    // The new question's own card comes one look later.
    crate::snapshot::PINNED_CLOCK.set(Some(now + 2000));
    let _question = question(&core, 7, now + 2000);
    let with_card = core.request("request_snapshot", json!({})).unwrap();
    core.requests().settle_native(&old);
    core.request("request_snapshot", json!({})).unwrap();
    let again = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(
        needs_you(&with_card).len(),
        2,
        "the new question has sounded"
    );
    assert_eq!(needs_you(&again).len(), 2);
}

#[test]
fn a_new_question_in_the_transcript_beside_an_older_permission_sounds() {
    question_beside_an_older_permission(true);
}

#[test]
fn a_new_question_card_after_its_transcript_sighting_is_not_left_silent() {
    question_beside_an_older_permission(false);
}

/// A native card naming its call (`callId`) is that call: when the rollout
/// shows it beside a new question, the new question sounds, and its card
/// after it adds no second sound. The third verifier's
/// `round3_core_named_native_card_cannot_absorb_two_sightings`.
#[test]
fn a_named_native_card_stands_for_no_second_sighting() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let (mut core, folder) = codex_core(temp.path());
    let view = json!({"source":"codex","sessionId":THREAD,"kind":"question","title":"Known question","callId":"known-question"});
    let _known = core
        .requests()
        .register_native("named-native-card".into(), view, now)
        .unwrap();
    let first = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(needs_you(&first).len(), 1);
    crate::snapshot::PINNED_CLOCK.set(Some(now + 1000));
    write_rollout_calls(
        &folder,
        now,
        &["known-question", "brand-new-question"],
        now + 1000,
    );
    let takeover = core.request("refresh", json!({})).unwrap();
    assert_eq!(row(&takeover)[0]["evidence"], Value::Null);
    assert_eq!(
        needs_you(&takeover).len(),
        2,
        "brand-new-question is a new question"
    );
    let _new = question(&core, 9, now + 2000);
    let later = core.request("request_snapshot", json!({})).unwrap();
    let again = core.request("request_snapshot", json!({})).unwrap();
    assert_eq!(needs_you(&later).len(), 2);
    assert_eq!(needs_you(&again).len(), 2);
}
