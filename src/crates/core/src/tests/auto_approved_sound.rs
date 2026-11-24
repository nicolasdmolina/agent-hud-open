//! Approve automatically against the fork's sound rule, through the whole
//! core (ticket 66). A permission allowed with no card makes no
//! `needs-you` where its channel says the session's wait is for that
//! request, by its id. Nothing else is silent: a wait that names no
//! request always sounds, as before the sync, though a request was just
//! approved. Each case of the two verifiers is a test here.
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
    /// The channel tells the session as waiting and names no request: all
    /// a channel knows of a wait it has no card for.
    fn told_waiting(&self) {
        self.core.requests().set_native_waiting(
            "codex",
            SOLE_READER,
            &[THREAD.to_owned()],
            self.now,
        );
    }
    /// The channel tells the session as waiting for these requests, by the
    /// ids it registered them under, as Codex Desktop's does.
    fn told_waiting_for(&self, requests: &[&str]) {
        let requests = requests.iter().map(|id| (*id).to_owned()).collect();
        self.core.requests().set_native_waits(
            "codex",
            SOLE_READER,
            &[(THREAD.to_owned(), requests)],
        );
    }
    /// The channel says the session waits no more.
    fn not_waiting(&self) {
        self.core
            .requests()
            .set_native_waiting("codex", SOLE_READER, &[], self.now);
    }
    /// The names of the approved requests core still keeps.
    fn kept(&self) -> Vec<String> {
        let kept = self.core.requests().auto_approved(self.now);
        let mut names: Vec<String> = kept.into_values().flatten().collect();
        names.sort();
        names
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
        // As the host plays them: each alert once, by its id.
        let needs: HashSet<&str> = snapshot["soundAlerts"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|alert| alert["kind"] == "needs-you")
            .filter_map(|alert| alert["id"].as_str())
            .collect();
        (state, cards, needs.len())
    }
    fn later(&mut self, ms: i64) {
        self.now += ms;
        crate::snapshot::PINNED_CLOCK.set(Some(self.now));
    }
}

const A: &str = "codex-desktop-permission:approved";

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
    assert!(!lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// The rule: the channel says the wait is for the request that was
/// allowed, so it is known to be that request's, and is silent.
#[test]
fn a_permission_allowed_automatically_shows_no_card_and_its_wait_plays_no_sound() {
    let mut lab = Lab::new(true);
    assert_eq!(
        lab.core
            .requests()
            .auto_approves("codex", Some("codex-desktop")),
        AutoApprove::from_settings(&json!({"autoApprove": true}))
            .approves("codex", Some("codex-desktop"))
    );
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 0),
        "a request that showed no card made a sound"
    );
    // The channel still says so a while later: the same wait, still silent.
    lab.later(6_000);
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
    // A question then asked shows its card, and that sounds.
    let question = "codex-desktop-question:2";
    assert!(!lab.card(question, "question"));
    lab.told_waiting_for(&[A, question]);
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// Round 1's first probe, with the wait named: the app first sees the
/// approved request's wait five seconds and a millisecond after allowing
/// it, and a minute after. No clock makes it sound.
#[test]
fn the_approved_requests_wait_is_silent_however_late_it_is_first_seen() {
    for late in [5_001, 60_000] {
        let mut lab = Lab::new(true);
        assert_eq!(lab.look(), ("running".into(), 0, 0));
        assert!(lab.card(A, "permission"));
        lab.told_waiting_for(&[A]);
        lab.later(late);
        assert_eq!(lab.look(), ("waiting".into(), 0, 0), "seen {late} ms late");
    }
}

/// Round 1's second probe. After the approval the session is seen at
/// work, with nothing waiting. A wait it then reports, naming no request,
/// is another wait and sounds though it has no card yet.
#[test]
fn a_separate_wait_after_an_automatic_approval_still_sounds() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.told_waiting();
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 1),
        "a new wait was taken for the approved request's"
    );
}

/// Round 2's first probe. The approved request's wait comes and goes
/// before any look, and another wait, for input the channel has no card
/// for, begins. The first look finds only that one. It sounds, whenever
/// the look comes: once, and not again when its card arrives.
#[test]
fn a_distinct_wait_that_names_no_request_sounds_before_the_first_look() {
    for first_look_after in [200, 6_000] {
        let mut lab = Lab::new(true);
        assert_eq!(lab.look(), ("running".into(), 0, 0));
        assert!(lab.card(A, "permission"));
        lab.told_waiting_for(&[A]);
        lab.later(100);
        lab.not_waiting();
        lab.later(100);
        lab.told_waiting();
        lab.later(first_look_after - 200);
        let at = format!("first look at {first_look_after} ms");
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{at}");
        lab.later(15_000);
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{at}, 15 s on");
        assert!(!lab.card("codex-desktop-question:late-card", "question"));
        assert_eq!(lab.look(), ("waiting".into(), 1, 1), "{at}, its card");
    }
}

/// Round 2's second probe. A wait the app already sees has sounded, and a
/// later answer cannot take that back: one sound, not two, whether or not
/// the channel then names the request.
#[test]
fn a_wait_seen_before_its_request_was_approved_has_sounded_once() {
    for named in [false, true] {
        let mut lab = Lab::new(true);
        assert_eq!(lab.look(), ("running".into(), 0, 0));
        lab.told_waiting();
        assert_eq!(lab.look(), ("waiting".into(), 0, 1));
        lab.later(100);
        assert!(lab.card(A, "permission"));
        if named {
            lab.told_waiting_for(&[A]);
        }
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "named: {named}");
        lab.later(6_000);
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "named: {named}");
    }
}

/// Round 2's third probe. Codex Desktop registers a request and then
/// publishes its wait; a look lands between the two. The request is still
/// kept after that look, so the wait that names it is silent.
#[test]
fn a_look_between_an_approval_and_the_publication_of_its_wait_stays_silent() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    assert_eq!(lab.kept(), [A]);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert_eq!(lab.kept(), [A], "a look that saw no wait of it ended it");
    lab.later(100);
    lab.told_waiting_for(&[A]);
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 0),
        "the approved request's own wait made a sound"
    );
}

/// Round 2's fourth probe, by the calls a look makes. The look reads the
/// approved requests, another is approved in the same session while it
/// decides, and then it says which waits it saw end. Only those go.
#[test]
fn an_approval_made_during_a_look_is_kept() {
    let lab = Lab::new(true);
    let requests = lab.core.requests();
    let session = format!("codex:{THREAD}");
    let first = "codex-desktop-permission:first";
    let second = "codex-desktop-permission:second";
    assert!(lab.card(first, "permission"));
    let read = requests.auto_approved(lab.now);
    assert_eq!(read[&session], [first]);
    assert!(lab.card(second, "permission"));
    requests.auto_approved_ended(&session, &read[&session]);
    assert_eq!(requests.auto_approved(lab.now)[&session], [second]);
}

/// A request is kept until a look sees the wait that names it end.
#[test]
fn an_approved_request_is_kept_until_a_look_sees_its_wait_end() {
    let mut lab = Lab::new(true);
    lab.look();
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
    assert_eq!(lab.kept(), [A]);
    lab.later(1_000);
    lab.not_waiting();
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.kept().is_empty());
}

/// What changed in round 3. A wait that names no request always sounds,
/// as it did before the sync, though a request was just approved in its
/// session: nothing says the wait is that request's. An extra sound for an
/// approved request is the lesser fault; a silent wait is not.
#[test]
fn a_wait_that_names_no_request_sounds_though_a_request_was_just_approved() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// A wait for the approved request and for something else beside it is
/// not only the request's: it sounds.
#[test]
fn a_wait_for_the_approved_request_and_another_sounds() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A, "codex-desktop-permission:no-card-yet"]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// The same from a hook's callback (`platform/src/hooks/callback.rs`). A
/// hook's request is known by its tool call, and the wait a hook reports
/// names none, so it sounds: the changed expectation of round 3 again.
#[test]
fn the_wait_a_hook_reports_after_an_automatic_approval_names_no_request_and_sounds() {
    let mut lab = Lab::new(true);
    lab.look();
    lab.core
        .requests()
        .note_auto_approved("codex", THREAD, Some("call-66"), lab.now);
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
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

/// The round 3 verifier's four looks. A wait that names no request sounds.
/// The session is seen at work. An approved request's wait then passes in
/// silence. A new wait that names no request begins, with nothing new in
/// the transcript, so the time it would be known by is the first wait's.
/// It is another wait, seen to begin after the first was seen to end: it
/// sounds, a second time.
#[test]
fn a_later_wait_that_names_no_request_sounds_though_nothing_new_was_written() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.later(100);
    lab.not_waiting();
    assert_eq!(lab.look(), ("running".into(), 0, 1));
    lab.later(100);
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.later(100);
    lab.not_waiting();
    lab.told_waiting();
    assert_eq!(
        lab.look(),
        ("waiting".into(), 0, 2),
        "a new wait was taken for one that had ended"
    );
    // The same wait at the next look: no third sound.
    lab.later(100);
    assert_eq!(lab.look(), ("waiting".into(), 0, 2));
}

/// The same with no approval in it at all: a wait, the session seen at
/// work, a wait again, and nothing new written between. Two waits, two
/// sounds. A wait that only goes on sounds once.
#[test]
fn a_wait_seen_to_end_and_begin_again_sounds_again() {
    let mut lab = Lab::new(false);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.later(100);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.not_waiting();
    assert_eq!(lab.look(), ("running".into(), 0, 1));
    lab.told_waiting();
    assert_eq!(lab.look(), ("waiting".into(), 0, 2));
}

/// The round 3 verifier's acknowledgment probe. A look read an approved
/// request of one session. Before it says that request's wait ended,
/// another source registers a request under the very same id, in a session
/// of its own, and it is allowed too. Ending the first ends it in its own
/// source and session only: the second, which no look has seen, is kept.
#[test]
fn ending_an_approved_request_ends_it_in_its_own_session_only() {
    let lab = Lab::new(true);
    let requests = lab.core.requests();
    let first = format!("codex:{THREAD}");
    let reused = "fixture-native-id";
    assert!(lab.card(reused, "permission"));
    let read = requests.auto_approved(lab.now);
    assert_eq!(read[&first], [reused]);
    requests.settle_native(reused);
    let other = json!({"source":"claude","sessionId":"second","kind":"permission"});
    let ticket = requests
        .register_native(reused.to_owned(), other, lab.now + 1)
        .unwrap();
    assert!(ticket.response.try_recv().ok().flatten().is_some());
    assert_eq!(
        requests.auto_approved(lab.now + 1)["claude:second"],
        [reused]
    );
    requests.auto_approved_ended(&first, &read[&first]);
    let kept = requests.auto_approved(lab.now + 1);
    assert_eq!(kept.get("claude:second"), Some(&vec![reused.to_owned()]));
    assert!(!kept.contains_key(&first));
}
