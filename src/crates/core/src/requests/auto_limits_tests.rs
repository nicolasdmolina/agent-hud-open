//! What Approve automatically never does (ticket 66): decide a request
//! marked `unreachable`, answer a request of a client whose requests are
//! answered by hand (Hermes), or leave a session it approved in without
//! a note for the sound decision.
use super::super::{auto, AutoApprove, Requests};
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
    assert_eq!(requests.auto_approved().len(), 1, "only the plain one");
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
    assert!(requests.auto_approved().is_empty());
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
/// its own names, from either way in, until that decision has looked at
/// its session. No clock ends it.
#[test]
fn an_automatic_approval_is_kept_by_its_names_until_its_session_is_looked_at() {
    let requests = approving();
    let mut card = permission("codex", Some("codex-desktop"));
    card["callId"] = json!("call-7");
    let ticket = requests.register_native("p".into(), card, 10_000).unwrap();
    assert_eq!(ticket.response.try_recv().unwrap(), Some(auto::allow()));
    requests.note_auto_approved("claude", "s9", Some("toolu_9"));
    requests.note_auto_approved("claude", "s9", None);
    let kept = requests.auto_approved();
    assert_eq!(kept["codex:s1"], ["p", "call:call-7"]);
    assert_eq!(kept["claude:s9"], ["call:toolu_9"]);
    assert_eq!(kept.len(), 2);
    // Time passing ends nothing: only a look at the session does.
    requests.expire(10_000_000);
    assert_eq!(requests.auto_approved(), kept);
    requests.auto_approved_seen(["codex:s1", "codex:another"]);
    let kept = requests.auto_approved();
    assert!(!kept.contains_key("codex:s1"));
    assert!(kept.contains_key("claude:s9"));
    requests.auto_approved_seen(["claude:s9"]);
    assert!(requests.auto_approved().is_empty());
    // A question is never approved, so never kept.
    let question = json!({"source": "codex", "client": "codex-desktop",
        "sessionId": "s2", "kind": "question"});
    let ticket = requests
        .register_native("q".into(), question, 20_000)
        .unwrap();
    assert!(ticket.response.try_recv().is_err());
    assert!(requests.auto_approved().is_empty());
}
