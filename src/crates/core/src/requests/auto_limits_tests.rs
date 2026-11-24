//! What Approve automatically never does (ticket 66): decide a request
//! marked `unreachable`, answer a request of a client whose requests are
//! answered by hand (Hermes), or leave a session it approved in without
//! a note for the sound decision.
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
    let ticket = requests
        .register_native("plain".into(), card.clone(), 1_000)
        .unwrap();
    assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
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
    assert_eq!(requests.auto_approved(1_000).len(), 1, "only the plain one");
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
    let ticket = requests.register_native("p".into(), card, 10_000).unwrap();
    assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
    requests.note_auto_approved("claude", "s9", Some("toolu_9"), 10_000);
    // A hook's request that names no tool call has nothing to be known by.
    requests.note_auto_approved("claude", "s9", None, 10_000);
    let kept = requests.auto_approved(10_000);
    assert_eq!(kept["codex:s1"], ["p", "call:call-7"]);
    assert_eq!(kept["claude:s9"], ["call:toolu_9"]);
    assert_eq!(kept.len(), 2);
    // A second approval in the same session, after the first was read.
    let other = permission("codex", Some("codex-desktop"));
    let ticket = requests
        .register_native("p2".into(), other, 10_001)
        .unwrap();
    assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
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
