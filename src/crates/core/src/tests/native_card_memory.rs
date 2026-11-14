//! What a card was stays with it for its life (ticket 41, after its fourth
//! verifier): a card a call once named, by its `callId` or by the one
//! possible pairing, is that call even when a later view of it names none,
//! and stands for no other sighting at a takeover. The fourth verifier's
//! core probes (`verifier-4/core-probes.rs`), with Codex Desktop's cards
//! built as its parser makes them.
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

fn sound_on(temp: &Path) -> AgentHudCore {
    let mut core = AgentHudCore::new(vec![], temp.join("app"));
    core.apply_setting("sound", &json!(true)).unwrap();
    show(&mut core);
    core
}
fn show(core: &mut AgentHudCore) -> Value {
    core.request("request_snapshot", json!({})).unwrap()
}
fn needs(snapshot: &Value) -> usize {
    snapshot["soundAlerts"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|alert| alert["kind"] == "needs-you")
        .count()
}
fn evidence(snapshot: &Value, id: &str) -> Value {
    snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()["evidence"]
        .clone()
}
/// A Codex rollout of session `id` whose open calls are `calls`, all
/// `request_user_input_async`, laid into Core as a read log.
fn rollout(core: &mut AgentHudCore, id: &str, calls: &[&str], now: i64) -> PathBuf {
    let mut log = Log::new(format!("synthetic/codex-{id}.jsonl").into(), "codex");
    log.parse(&json!({"timestamp":iso(now-3000),"type":"session_meta","payload":{"id":id,"source":"vscode","cwd":"/synthetic"}}));
    log.parse(&json!({"timestamp":iso(now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-41"}}));
    for call in calls {
        log.parse(&json!({"timestamp":iso(now),"type":"response_item","payload":{"type":"function_call","call_id":call,"name":"request_user_input_async","arguments":"{}"}}));
    }
    let head = LogHead::from_log(&log);
    let path = head.path.clone();
    core.logs.insert(path.clone(), head);
    forget_projection(core);
    path
}
fn forget_projection(core: &mut AgentHudCore) {
    core.usage_projection = None;
    core.sessions_checked = None;
    core.statistics.revision += 1;
}
/// A card of `kind` in session `id`, naming `call` where given.
fn card(
    core: &AgentHudCore,
    id: &str,
    card: &str,
    call: Option<&str>,
    now: i64,
) -> requests::Ticket {
    let mut view =
        json!({"source":"codex","sessionId":id,"kind":"question","title":"Synthetic question"});
    if let Some(call) = call {
        view["callId"] = json!(call);
    }
    core.requests()
        .register_native(card.into(), view, now)
        .unwrap()
}

/// A named and an unnamed card, the log twice read and once lost between:
/// the unnamed card the first takeover paired with `b` is `b` at the second,
/// so `brand-new` sounds. The fourth verifier's
/// `round4_core_mixed_cards_two_takeovers_must_not_silence_a_new_question`.
#[test]
fn a_card_paired_at_the_first_takeover_leaves_a_new_question_audible_at_the_second() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let mut core = sound_on(temp.path());
    let id = "00000000-0000-4000-8000-000000004041";
    let _named = card(&core, id, "r4-named", Some("a"), now);
    let _unnamed = card(&core, id, "r4-unnamed", None, now);
    assert_eq!(needs(&show(&mut core)), 1);
    let path = rollout(&mut core, id, &["a", "b"], now);
    assert_eq!(needs(&show(&mut core)), 1);
    core.logs.remove(&path);
    forget_projection(&mut core);
    let fallback = show(&mut core);
    assert_eq!(evidence(&fallback, id), "native-card");
    rollout(&mut core, id, &["a", "b", "brand-new"], now);
    let second = show(&mut core);
    assert_eq!(needs(&second), 2, "brand-new is a call of its own");
    assert_eq!(needs(&show(&mut core)), 2);
}

/// A card whose refreshed view drops its `callId` is still that call. The
/// fourth verifier's
/// `round4_core_refresh_omitting_a_known_call_id_must_not_silence_a_distinct_question`.
#[test]
fn a_refresh_that_drops_a_cards_call_id_leaves_a_new_question_audible() {
    let temp = tempfile::tempdir().unwrap();
    let now = Utc::now().timestamp_millis();
    let _clock = Clock::at(now);
    let mut core = sound_on(temp.path());
    let id = "00000000-0000-4000-8000-000000004043";
    let _ticket = card(&core, id, "r4-refreshed", Some("a"), now);
    assert_eq!(needs(&show(&mut core)), 1);
    let view = json!({"source":"codex","sessionId":id,"kind":"question","title":"Synthetic question","detail":"Updated"});
    core.requests().refresh_native("r4-refreshed", view);
    rollout(&mut core, id, &["a", "brand-new"], now);
    let takeover = show(&mut core);
    assert_eq!(
        needs(&takeover),
        2,
        "the card is a; brand-new is a new question"
    );
    assert_eq!(needs(&show(&mut core)), 2);
}
