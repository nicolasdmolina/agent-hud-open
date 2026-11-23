//! Approve automatically against the fork's sound rule, through the whole
//! core (ticket 66): a permission allowed with no card makes no
//! `needs-you`, though its channel tells the session as waiting for a
//! moment; the same wait with nothing approved sounds, and so does a card
//! that shows afterwards.
use super::*;
use crate::requests::AutoApprove;
use agent_hud_agents::SOLE_READER;

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

const THREAD: &str = "00000000-0000-4000-8000-000000000066";

struct Lab {
    core: AgentHudCore,
    now: i64,
    _temp: tempfile::TempDir,
    _clock: Clock,
}

impl Lab {
    /// A Codex Desktop session at work, sound on, past the first look.
    fn new(auto_approve: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now().timestamp_millis();
        let clock = Clock::at(now);
        let folder = temp.path().join(".codex/sessions");
        fs::create_dir_all(&folder).unwrap();
        let lines = [
            json!({"timestamp":crate::iso(now-3000),"type":"session_meta","payload":{"id":THREAD,"timestamp":crate::iso(now-3000),"source":"vscode","originator":"Codex Desktop","cwd":"/synthetic"}}),
            json!({"timestamp":crate::iso(now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-66"}}),
        ];
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(folder.join(format!("rollout-{THREAD}.jsonl")), text).unwrap();
        let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
        core.provider_roots = vec![(folder, "codex")];
        core.apply_setting("sound", &json!(true)).unwrap();
        core.apply_setting("autoApprove", &json!(auto_approve))
            .unwrap();
        core.request("refresh", json!({})).unwrap();
        Self {
            core,
            now,
            _temp: temp,
            _clock: clock,
        }
    }
    /// A card as the Desktop parser makes it; whether it was allowed at
    /// once, with no card.
    fn card(&self, id: &str, kind: &str) -> bool {
        let view = json!({"source":"codex","client":"codex-desktop","vendor":"Codex","sessionId":THREAD,"title":"Synthetic","summary":"Synthetic","toolName":"shell","kind":kind,"input":"","detail":null,"cwd":"/synthetic","createdAt":crate::iso(self.now),"allowsAlways":false,"answersQuestion":kind=="question","answersPlan":false,"plan":null,"suggestions":[],"questions":[]});
        let ticket = self
            .core
            .requests()
            .register_native(id.into(), view, self.now)
            .unwrap();
        let allowed = ticket.response.try_recv().ok().flatten().is_some();
        std::mem::forget(ticket);
        allowed
    }
    /// The channel tells the session as waiting, as it does while its
    /// client asks.
    fn told_waiting(&self) {
        self.core.requests().set_native_waiting(
            "codex",
            SOLE_READER,
            &[THREAD.to_owned()],
            self.now,
        );
    }
    /// One look: (the session's state, cards shown, `needs-you` alerts).
    fn look(&mut self) -> (String, usize, usize) {
        let snapshot = self.core.request("request_snapshot", json!({})).unwrap();
        let state = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == THREAD)
            .and_then(|row| row["state"].as_str())
            .unwrap_or("absent")
            .to_owned();
        let cards = snapshot["approvals"].as_array().unwrap().len();
        let needs = snapshot["soundAlerts"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|alert| alert["kind"] == "needs-you")
            .count();
        (state, cards, needs)
    }
    fn later(&mut self, ms: i64) {
        self.now += ms;
        crate::snapshot::PINNED_CLOCK.set(Some(self.now));
    }
}

/// The control: a wait its channel tells, with no card and nothing
/// approved, sounds once.
#[test]
fn a_wait_told_with_no_card_sounds_where_nothing_was_approved() {
    let mut lab = Lab::new(false);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// With Approve automatically off the permission shows its card and
/// sounds, as before.
#[test]
fn a_permission_that_shows_its_card_sounds() {
    let mut lab = Lab::new(false);
    lab.look();
    assert!(!lab.card("codex-desktop-permission:1", "permission"));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

#[test]
fn a_permission_allowed_automatically_shows_no_card_and_plays_no_sound() {
    let mut lab = Lab::new(true);
    assert_eq!(
        lab.core
            .requests()
            .auto_approves("codex", Some("codex-desktop")),
        AutoApprove::from_settings(&json!({"autoApprove": true}))
            .approves("codex", Some("codex-desktop"))
    );
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    // Its client asks: the channel tells the wait and hands the card over,
    // which is allowed at once.
    lab.told_waiting();
    assert!(lab.card("codex-desktop-permission:1", "permission"));
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 0),
        "a request that showed no card made a sound"
    );
    // The channel still says so a while later: the same wait, still silent.
    lab.later(6_000);
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
    // A question then asked shows its card, and that sounds.
    assert!(!lab.card("codex-desktop-question:2", "question"));
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// The same from a hook's callback, which tells core the session it
/// allowed in (`platform/src/hooks/callback.rs`).
#[test]
fn a_permission_a_hook_allowed_automatically_plays_no_sound() {
    let mut lab = Lab::new(true);
    lab.look();
    lab.core
        .requests()
        .note_auto_approved("codex", THREAD, Some("call-66"));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
}

/// Upstream's "Answer in the HUD", turned off: the card is left out of the
/// panel, and the session still waits on the person in its own client, so
/// it still sounds. That is the fork's sound rule (a session that starts
/// waiting), not a request allowed with no card.
#[test]
fn a_request_answered_in_its_own_client_still_sounds() {
    let mut lab = Lab::new(false);
    lab.core
        .apply_setting("answerInHud", &json!(false))
        .unwrap();
    lab.look();
    assert!(!lab.card("codex-desktop-permission:1", "permission"));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// The verifier's first probe (round 2). The app first sees the approved
/// request's wait five seconds and a millisecond after allowing it: no
/// look came in between. It is still that request's wait, however late it
/// is seen, and makes no sound.
#[test]
fn a_wait_first_seen_long_after_its_automatic_approval_plays_no_sound() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card("codex-desktop-permission:1", "permission"));
    lab.told_waiting();
    lab.later(5_001);
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 0),
        "a request that showed no card made a sound, seen late"
    );
}

/// The verifier's second probe. After the approval the session is seen at
/// work, with nothing waiting. A wait it then reports is another wait, not
/// the approved request's, and sounds though it has no card yet.
#[test]
fn a_separate_wait_after_an_automatic_approval_still_sounds() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card("codex-desktop-permission:1", "permission"));
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.told_waiting();
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 1),
        "a new wait was taken for the approved request's"
    );
}
