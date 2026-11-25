//! What Approve automatically never does (ticket 66): decide a request
//! marked `unreachable`, answer a request of a client whose requests are
//! answered by hand (Hermes), or count a request as approved for sound
//! before its channel confirmed that the answer was delivered.
use super::super::{auto, AutoApprove, Requests};
use super::{APPROVED_KEPT, APPROVED_KEPT_MS};
use serde_json::{json, Value};

const WHY: &str = "The client cannot be reached right now.";

fn approving() -> Requests {
    let requests = Requests::default();
    requests.set_auto_approve(AutoApprove::from_settings(&json!({"autoApprove": true})));
    requests
}

fn permission(source: &str, client: Option<&str>) -> Value {
    let mut view = json!({"source": source, "sessionId": "s1", "kind": "permission",
        "title": "Synthetic", "toolName": "Bash", "allowsAlways": false});
    if let Some(client) = client {
        view["client"] = json!(client);
    }
    view
}

/// Registers the card under `id`, which is allowed at once: the wait its
/// answer was given in, which is what its channel confirms it by.
fn allowed(requests: &Requests, id: &str, card: &Value, now_ms: i64) -> u64 {
    let ticket = requests.register_native(id.into(), card.clone(), now_ms);
    let ticket = ticket.unwrap();
    assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
    ticket.wait
}

fn shown(requests: &Requests) -> Vec<String> {
    requests
        .views(1_000)
        .iter()
        .filter_map(|view| view["id"].as_str().map(str::to_owned))
        .collect()
}

/// The same card, with and without its mark: unmarked it is allowed at
/// once; marked it shows, waits, and hears nothing.
#[test]
fn approve_automatically_never_decides_a_request_marked_unreachable() {
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    let plain = allowed(&requests, "plain", &card, 1_000);
    assert!(shown(&requests).is_empty());

    let mut marked = card.clone();
    marked["unreachable"] = json!(WHY);
    let ticket = requests
        .register_native("marked".into(), marked, 1_000)
        .unwrap();
    assert!(
        ticket.response.try_recv().is_err(),
        "a request marked unreachable was decided"
    );
    assert_eq!(shown(&requests), ["marked"]);
    // It was not noted as approved, and takes no decision by hand either.
    for wait in [plain, ticket.wait] {
        requests.auto_answer_delivered(wait, 1_000);
    }
    let kept = requests.auto_approved(1_000);
    assert_eq!(kept["codex:s1"], ["plain"], "only the plain one");
    assert_eq!(
        requests.answer("marked", &json!({"kind": "allow"}), 1_500),
        Err(WHY.to_owned())
    );
    // Its mark gone, it still waits for the person: nothing decides a
    // card that already shows.
    requests.refresh_native("marked", card);
    assert!(ticket.response.try_recv().is_err());
    assert_eq!(shown(&requests), ["marked"]);
}

/// Hermes's requests are answered by hand: with Approve automatically on
/// for every client, an approval of Hermes's shows its card and hears
/// nothing, by the rule itself and at both ways in (a native card, and
/// what a hook's callback asks).
#[test]
fn approve_automatically_never_answers_a_hermes_request() {
    let rule = AutoApprove::from_settings(&json!({"autoApprove": true}));
    assert!(rule.approves("codex", None), "the rule is on");
    for client in [None, Some("hermes"), Some("Hermes Agent · reports")] {
        assert!(!rule.approves("hermes", client), "{client:?}");
    }
    let requests = approving();
    assert!(!requests.auto_approves("hermes", None));
    assert!(!requests.auto_approves("hermes", Some("hermes")));
    let ticket = requests
        .register_native("hermes:1".into(), permission("hermes", None), 1_000)
        .unwrap();
    assert!(
        ticket.response.try_recv().is_err(),
        "a Hermes approval was answered"
    );
    assert_eq!(shown(&requests), ["hermes:1"]);
    requests.note_native_delivery("hermes:1", "hermes", "s1", "confirmed", None, 1_000);
    assert!(requests.auto_approved(1_000).is_empty());
    // By hand it is answered as ever.
    assert!(requests
        .answer("hermes:1", &json!({"kind": "allow"}), 1_500)
        .is_ok());
}

/// Which agents are answered by hand is their own descriptor's word:
/// Hermes's, and no other agent's.
#[test]
fn only_hermes_is_answered_by_hand() {
    let by_hand: Vec<&str> = agent_hud_agents::ALL
        .iter()
        .map(|agent| agent.id)
        .filter(|id| super::by_hand(id))
        .collect();
    assert_eq!(by_hand, ["hermes"]);
}

/// A request allowed with no card is kept for the sound decision under
/// its own names, from either way in, and ends by its name only: when the
/// sound decision says the wait that went by it is over.
#[test]
fn an_automatic_approval_is_kept_by_its_names_and_ended_by_them() {
    let requests = approving();
    let mut card = permission("codex", Some("codex-desktop"));
    card["callId"] = json!("call-7");
    let wait = allowed(&requests, "p", &card, 10_000);
    requests.auto_answer_delivered(wait, 10_000);
    requests.note_auto_approved("claude", "s9", Some("toolu_9"), 10_000);
    // A hook's request that names no tool call has nothing to be known by.
    requests.note_auto_approved("claude", "s9", None, 10_000);
    let kept = requests.auto_approved(10_000);
    assert_eq!(kept["codex:s1"], ["p", "call:call-7"]);
    assert_eq!(kept["claude:s9"], ["call:toolu_9"]);
    assert_eq!(kept.len(), 2);
    // A second approval in the same session, after the first was read.
    let other = permission("codex", Some("codex-desktop"));
    let wait = allowed(&requests, "p2", &other, 10_001);
    requests.auto_answer_delivered(wait, 10_001);
    // Ending what was read ends that request and no other, though they
    // share a session: by one of its names, either of them.
    let ended = ["call:call-7".to_owned(), "unknown".to_owned()];
    // Not in another session, though the name is the same.
    requests.auto_approved_ended("claude:s9", &ended);
    assert_eq!(requests.auto_approved(10_002)["codex:s1"].len(), 3);
    requests.auto_approved_ended("codex:s1", &ended);
    let kept = requests.auto_approved(10_002);
    assert_eq!(kept["codex:s1"], ["p2"]);
    assert_eq!(kept["claude:s9"], ["call:toolu_9"]);
    requests.auto_approved_ended("codex:s1", &[]);
    assert_eq!(requests.auto_approved(10_002), kept);
    // A question is never approved, so never kept.
    let question = json!({"source": "codex", "client": "codex-desktop",
        "sessionId": "s2", "kind": "question"});
    let ticket = requests
        .register_native("q".into(), question, 20_000)
        .unwrap();
    assert!(ticket.response.try_recv().is_err());
    assert!(!requests.auto_approved(20_000).contains_key("codex:s2"));
}

/// What is kept is bounded, in time and in number, so that a request whose
/// wait is never seen to end cannot make the list grow. Ten minutes, and
/// 512 requests, the oldest going first.
#[test]
fn what_is_kept_of_approved_requests_is_bounded_in_time_and_number() {
    let requests = approving();
    assert_eq!(APPROVED_KEPT_MS, 600_000);
    assert_eq!(APPROVED_KEPT, 512);
    requests.note_auto_approved("claude", "s1", Some("old"), 1_000);
    requests.note_auto_approved("claude", "s1", Some("new"), 2_000);
    assert_eq!(
        requests.auto_approved(1_000 + APPROVED_KEPT_MS - 1)["claude:s1"],
        ["call:old", "call:new"]
    );
    assert_eq!(
        requests.auto_approved(1_000 + APPROVED_KEPT_MS)["claude:s1"],
        ["call:new"]
    );
    assert!(requests.auto_approved(2_000 + APPROVED_KEPT_MS).is_empty());
    for index in 0..APPROVED_KEPT + 3 {
        requests.note_auto_approved("claude", "s2", Some(&format!("c{index}")), 5_000);
    }
    let kept = &requests.auto_approved(5_000)["claude:s2"];
    assert_eq!(kept.len(), APPROVED_KEPT);
    assert_eq!(kept[0], "call:c3");
}

/// A wait names its requests only where every reader that lists the
/// session names them: a session any reader lists with none is waiting on
/// something more, and is known by no request.
#[test]
fn a_wait_is_for_its_requests_only_where_every_reader_names_them() {
    let requests = Requests::default();
    let waits = |list: &[(&str, &[&str])]| -> Vec<(String, Vec<String>)> {
        list.iter()
            .map(|(session, ids)| {
                let ids = ids.iter().map(|id| (*id).to_owned()).collect();
                ((*session).to_owned(), ids)
            })
            .collect()
    };
    requests.set_native_waits("codex", "one", &waits(&[("s1", &["r1"]), ("s2", &[])]));
    let named = requests.native_wait_requests();
    assert_eq!(named["codex:s1"], ["r1"]);
    assert!(!named.contains_key("codex:s2"), "a wait that names none");
    assert!(requests.native_waiting_sessions()["codex"].contains("s2"));
    // A second reader lists s1 too: with its own request, then with none.
    requests.set_native_waits("codex", "two", &waits(&[("s1", &["r2"])]));
    let mut both = requests.native_wait_requests()["codex:s1"].clone();
    both.sort();
    assert_eq!(both, ["r1", "r2"]);
    requests.set_native_waiting("codex", "two", &["s1".to_owned()], 0);
    assert!(!requests.native_wait_requests().contains_key("codex:s1"));
    // That reader ends: the first one's word stands again.
    requests.end_native_reader("codex", "two");
    assert_eq!(requests.native_wait_requests()["codex:s1"], ["r1"]);
    // The same list told again changes nothing; a new name does.
    let revision = requests.revision();
    requests.set_native_waits("codex", "one", &waits(&[("s1", &["r1"]), ("s2", &[])]));
    assert_eq!(requests.revision(), revision);
    requests.set_native_waits("codex", "one", &waits(&[("s1", &["r3"]), ("s2", &[])]));
    assert_ne!(requests.revision(), revision);
    requests.set_native_waiting("codex", "one", &[], 0);
    assert!(requests.native_wait_requests().is_empty());
}

/// An approval is only as good as its answer. A request allowed at once
/// is not among the approved until its channel says that the answer given
/// in its own wait was delivered. A receipt as the panel shows it, which
/// names the request by its id, says nothing either way, whatever it says
/// and for whichever session; nor does word of another wait's answer.
#[test]
fn an_automatic_approval_counts_only_once_its_own_answer_is_confirmed() {
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    let wait = allowed(&requests, "p", &card, 1_000);
    let other = allowed(&requests, "other", &card, 1_000);
    assert!(requests.auto_approved(1_000).is_empty(), "before any word");
    for word in ["failed", "uncertain", "confirmed"] {
        for (source, session) in [("codex", "s1"), ("codex", "another"), ("claude", "s1")] {
            requests.note_native_delivery("p", source, session, word, None, 1_500);
        }
    }
    assert!(requests.auto_approved(1_500).is_empty(), "a receipt by id");
    // A wait nothing was allowed in, and one after every wait there is.
    for unknown in [0, wait.max(other) + 1] {
        requests.auto_answer_delivered(unknown, 1_500);
    }
    assert!(requests.auto_approved(1_500).is_empty());
    requests.auto_answer_delivered(other, 2_000);
    assert_eq!(requests.auto_approved(2_000)["codex:s1"], ["other"]);
    requests.auto_answer_delivered(wait, 2_000);
    assert_eq!(requests.auto_approved(2_000)["codex:s1"], ["p", "other"]);
    // A failure told by id afterwards takes nothing back either.
    requests.note_native_delivery("p", "codex", "s1", "failed", None, 2_500);
    assert_eq!(requests.auto_approved(2_500)["codex:s1"].len(), 2);
    // It ends by its name, in its session.
    requests.auto_approved_ended("codex:s1", &["p".to_owned()]);
    assert_eq!(requests.auto_approved(2_500)["codex:s1"], ["other"]);
}

/// Round 8. A request that is settled and asked again under the same id
/// is another request, with a wait and an answer of its own. Word of the
/// first one's answer, which may come late, confirms nothing of it.
#[test]
fn word_of_a_settled_requests_answer_confirms_nothing_of_the_one_that_reuses_its_id() {
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    let first = allowed(&requests, "p", &card, 1_000);
    requests.settle_native("p");
    let second = allowed(&requests, "p", &card, 1_001);
    assert_ne!(first, second);
    requests.auto_answer_delivered(first, 1_002);
    requests.note_native_delivery("p", "codex", "s1", "confirmed", None, 1_002);
    assert!(
        requests.auto_approved(1_002).is_empty(),
        "the old answer confirmed the new request"
    );
    requests.auto_answer_delivered(second, 1_003);
    assert_eq!(requests.auto_approved(1_003)["codex:s1"], ["p"]);
}

/// Settling a native request ends its approval, confirmed or not: where
/// its listener stopped before the answer left, and where the request is
/// over in its client. A hook's approval, which no channel settles, and
/// another request's are left.
#[test]
fn settling_a_native_request_ends_its_approval() {
    for confirmed in [false, true] {
        let requests = approving();
        let mut card = permission("codex", Some("codex-desktop"));
        card["callId"] = json!("call-7");
        let wait = allowed(&requests, "p", &card, 1_000);
        let other = allowed(&requests, "other", &card, 1_000);
        requests.auto_answer_delivered(other, 1_000);
        requests.note_auto_approved("codex", "s1", Some("p"), 1_000);
        if confirmed {
            requests.auto_answer_delivered(wait, 1_000);
        }
        requests.settle_native("p");
        // Confirmed after it was settled: there is nothing to confirm.
        requests.auto_answer_delivered(wait, 1_001);
        assert_eq!(
            requests.auto_approved(1_001)["codex:s1"],
            ["other", "call:call-7", "call:p"],
            "confirmed before: {confirmed}"
        );
        // Shown again, it is a request like any other.
        requests.set_auto_approve(AutoApprove::from_settings(&json!({})));
        let ticket = requests.register_native("p".into(), card, 1_002).unwrap();
        assert!(ticket.response.try_recv().is_err());
        assert_eq!(shown(&requests), ["p"]);
    }
}

/// A confirmed approval is kept past ten minutes only while a wait of its
/// own session is said to be for it; one not confirmed is never kept
/// longer, wait or no wait.
#[test]
fn a_confirmed_approval_is_kept_while_its_wait_is_named_for_it() {
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    let ids = ["named", "unnamed", "elsewhere", "unconfirmed"];
    let waits_of = ids.map(|id| allowed(&requests, id, &card, 1_000));
    for wait in &waits_of[..3] {
        requests.auto_answer_delivered(*wait, 1_000);
    }
    let waits = |list: &[(&str, &[&str])]| -> Vec<(String, Vec<String>)> {
        let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect();
        list.iter()
            .map(|(session, names)| ((*session).to_owned(), ids(names)))
            .collect()
    };
    // Another session's wait names `elsewhere`; s1's names the other two.
    let told = waits(&[("s1", &["named", "unconfirmed"]), ("s2", &["elsewhere"])]);
    requests.set_native_waits("codex", "reader", &told);
    let late = 1_000 + APPROVED_KEPT_MS;
    assert_eq!(requests.auto_approved(late - 1)["codex:s1"].len(), 3);
    assert_eq!(requests.auto_approved(late)["codex:s1"], ["named"]);
    // Too late to confirm the one that was not.
    requests.auto_answer_delivered(waits_of[3], late);
    assert_eq!(requests.auto_approved(late)["codex:s1"], ["named"]);
    // The wait names it no more: it goes.
    requests.set_native_waits("codex", "reader", &waits(&[("s1", &[])]));
    assert!(requests.auto_approved(late + 1).is_empty());
}

/// Round 7. A request not heard of in ten minutes is approved no more, and
/// the first word of its answer that comes after confirms nothing, though
/// nothing asked core in between: the word itself is held to the age.
#[test]
fn a_confirmation_that_comes_after_ten_minutes_confirms_nothing() {
    for (late, approved) in [(APPROVED_KEPT_MS - 1, true), (APPROVED_KEPT_MS, false)] {
        let requests = approving();
        let card = permission("codex", Some("codex-desktop"));
        let wait = allowed(&requests, "p", &card, 1_000);
        let waits = [("s1".to_owned(), vec!["p".to_owned()])];
        requests.set_native_waits("codex", "reader", &waits);
        requests.auto_answer_delivered(wait, 1_000 + late);
        let kept = requests.auto_approved(1_000 + late);
        assert_eq!(kept.contains_key("codex:s1"), approved, "{late} ms");
        // Confirmed in time, a second word long after changes nothing.
        requests.auto_answer_delivered(wait, 1_000 + 3 * APPROVED_KEPT_MS);
        let kept = requests.auto_approved(1_000 + 3 * APPROVED_KEPT_MS);
        assert_eq!(kept.contains_key("codex:s1"), approved, "{late} ms, later");
    }
}

/// Round 9. Where its channel lost sight of a request, nothing known of
/// its automatic answer is carried over: the approval goes, confirmed or
/// not, and so does the dismissal, so the request the channel sees next
/// under that id is registered as new and answered anew, in a wait of its
/// own. Word of the first answer confirms nothing of it. A request that
/// was answered by hand keeps its dismissal, as before the sync, and so
/// does one the switch never touched.
#[test]
fn a_request_its_channel_lost_sight_of_is_registered_and_answered_anew() {
    for confirmed in [false, true] {
        let requests = approving();
        let card = permission("codex", Some("codex-desktop"));
        let first = allowed(&requests, "p", &card, 1_000);
        let other = allowed(&requests, "other", &card, 1_000);
        requests.auto_answer_delivered(other, 1_000);
        if confirmed {
            requests.auto_answer_delivered(first, 1_000);
        }
        // Registered again while it is known: the same request, no ticket.
        assert!(requests
            .register_native("p".into(), card.clone(), 1_001)
            .is_none());
        requests.auto_approval_lost("p");
        assert_eq!(requests.auto_approved(1_001)["codex:s1"], ["other"]);
        let second = allowed(&requests, "p", &card, 1_002);
        assert_ne!(first, second);
        requests.auto_answer_delivered(first, 1_003);
        assert_eq!(requests.auto_approved(1_003)["codex:s1"], ["other"]);
        requests.auto_answer_delivered(second, 1_004);
        assert_eq!(requests.auto_approved(1_004)["codex:s1"], ["other", "p"]);
        assert!(shown(&requests).is_empty());
    }
    // Answered by hand: lost sight of, it is still dismissed.
    let requests = Requests::default();
    let card = permission("codex", Some("codex-desktop"));
    // The ticket is held, as its channel holds it until it is answered.
    let ticket = requests.register_native("p".into(), card.clone(), 1_000);
    let ticket = ticket.unwrap();
    assert!(ticket.response.try_recv().is_err());
    assert!(requests
        .answer("p", &json!({"kind": "allow"}), 1_001)
        .is_ok());
    requests.auto_approval_lost("p");
    assert!(requests.register_native("p".into(), card, 1_002).is_none());
    // An id that was allowed at once, settled, and then answered by hand.
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    allowed(&requests, "q", &card, 1_000);
    requests.settle_native("q");
    requests.set_auto_approve(AutoApprove::from_settings(&json!({})));
    // The ticket is held, as its channel holds it until it is answered.
    let ticket = requests.register_native("q".into(), card.clone(), 1_001);
    let ticket = ticket.unwrap();
    assert!(ticket.response.try_recv().is_err());
    assert!(requests
        .answer("q", &json!({"kind": "allow"}), 1_002)
        .is_ok());
    requests.auto_approval_lost("q");
    assert!(requests.register_native("q".into(), card, 1_003).is_none());
}

/// Antigravity's and DeepSeek's channels: with Approve automatically on
/// their permissions are allowed at once too, but neither names the
/// requests a wait is for, and neither tells core of an answer by its
/// wait. So nothing of theirs is ever approved for sound, whatever
/// receipts they report, and their waits always sound.
#[test]
fn a_channel_that_never_tells_core_by_the_wait_has_nothing_approved() {
    for source in ["antigravity", "deepseek"] {
        let requests = approving();
        assert!(requests.auto_approves(source, None), "{source}");
        let card = json!({"source": source, "sessionId": "s1", "kind": "permission"});
        let ticket = requests.register_native("p".into(), card, 1_000).unwrap();
        assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
        for word in ["confirmed", "failed", "uncertain"] {
            requests.note_native_delivery("p", source, "s1", word, None, 1_500);
        }
        requests.set_native_waiting(source, "reader", &["s1".to_owned()], 1_500);
        assert!(requests.auto_approved(1_500).is_empty(), "{source}");
        assert!(requests.native_wait_requests().is_empty(), "{source}");
    }
}

/// The look's word that an approved request's wait ended ends what the
/// look was told was approved. A request registered anew under that id
/// since, its answer not yet confirmed, is another request and is kept,
/// so that its own answer can still confirm it.
#[test]
fn the_end_of_an_approved_wait_does_not_end_a_request_registered_anew_under_its_id() {
    let requests = approving();
    let card = permission("codex", Some("codex-desktop"));
    let first = allowed(&requests, "p", &card, 1_000);
    requests.auto_answer_delivered(first, 1_000);
    let told = requests.auto_approved(1_000)["codex:s1"].clone();
    requests.auto_approval_lost("p");
    let second = allowed(&requests, "p", &card, 1_001);
    requests.auto_approved_ended("codex:s1", &told);
    requests.auto_answer_delivered(second, 1_002);
    assert_eq!(requests.auto_approved(1_002)["codex:s1"], ["p"]);
    // Once it is the approved one, its own wait's end does end it.
    requests.auto_approved_ended("codex:s1", &told);
    assert!(requests.auto_approved(1_002).is_empty());
}
