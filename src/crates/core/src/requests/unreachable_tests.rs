//! A request whose client cannot be reached: its card says so and takes no
//! answer, and a request whose answer could not be sent is put back as it
//! was, unless it came to its end meanwhile.
use super::Requests;
use serde_json::{json, Value};

const WHY: &str = "The client cannot be reached right now.";
const HOLD: i64 = 600_000;

fn question() -> Value {
    json!({"kind": "question", "source": "client", "sessionId": "s", "title": "t",
        "answersQuestion": true,
        "questions": [{"answerKey": "q", "question": "Which?", "multiSelect": false,
            "options": [{"label": "one", "description": null}]}]})
}

fn marked() -> Value {
    let mut view = question();
    view["unreachable"] = json!(WHY);
    view
}

fn pick() -> Value {
    json!({"kind": "answer", "answers": {"q": {"selected": ["one"]}}})
}

fn dismissed(requests: &Requests, id: &str, now: i64) -> bool {
    // A dismissed request is not registered again. Where this registers
    // one to find out, it takes it away again.
    let registered = requests
        .register_native(id.into(), question(), now)
        .is_some();
    if registered {
        requests.cancel(id);
    }
    !registered
}

/// The gate: a marked request refuses every answer, and nothing about it
/// changes. Its mark gone, it takes the same answer.
#[test]
fn a_request_marked_unreachable_takes_no_decision_and_stays_as_it_was() {
    let requests = Requests::default();
    let ticket = requests
        .register_native("native:1".into(), question(), 1_000)
        .unwrap();
    requests.refresh_native("native:1", marked());
    let before = (requests.views(2_000), requests.revision());
    for decision in [pick(), json!({"kind": "allow"}), json!({"kind": "deny"})] {
        assert_eq!(
            requests.answer("native:1", &decision, 2_000),
            Err(WHY.to_owned()),
            "{decision}"
        );
        assert_eq!(
            requests.checked("native:1", &decision, 2_000),
            Err(WHY.to_owned()),
            "{decision}"
        );
    }
    // Not removed, not dismissed, not answered, and its hold is where it was.
    assert_eq!((requests.views(2_000), requests.revision()), before);
    assert_eq!(requests.deadline("native:1"), Some(1_000 + HOLD));
    assert!(ticket.response.try_recv().is_err());
    // Marked with something that is no text, it is refused all the same.
    let mut odd = question();
    odd["unreachable"] = json!(true);
    requests.refresh_native("native:1", odd);
    assert!(requests.answer("native:1", &pick(), 2_000).is_err());
    assert_eq!(requests.views(2_000).len(), 1);

    requests.refresh_native("native:1", question());
    requests.answer("native:1", &pick(), 3_000).unwrap();
    assert_eq!(ticket.response.recv().unwrap(), Some(pick()));
}

/// Leaving a card alone sends nothing anywhere, so a marked card can still
/// be put away, and it stays away.
#[test]
fn a_marked_request_can_still_be_left_alone() {
    let requests = Requests::default();
    let ticket = requests
        .register_native("native:1".into(), marked(), 1_000)
        .unwrap();
    requests
        .answer("native:1", &json!({"kind": "leave"}), 2_000)
        .unwrap();
    assert_eq!(ticket.response.recv().unwrap(), None);
    assert!(requests.views(2_000).is_empty());
    assert!(dismissed(&requests, "native:1", 2_000));
    // Nothing was to be sent, so there is nothing to give back.
    assert!(requests
        .put_back("native:1", ticket.wait, marked(), 2_001)
        .is_none());
    assert!(requests.views(2_001).is_empty());
}

/// Marking and unmarking is a change in place: the same request, the same
/// time it came, the same deadline.
#[test]
fn a_mark_changes_a_request_in_place_and_moves_no_deadline() {
    let requests = Requests::default();
    requests
        .register_native("native:1".into(), question(), 1_000)
        .unwrap();
    let before = requests.views(1_500);
    requests.refresh_native("native:1", marked());
    assert_eq!(requests.views(5_000)[0]["unreachable"], WHY);
    assert_eq!(requests.deadline("native:1"), Some(1_000 + HOLD));
    assert_eq!(requests.waiting_at("client", "s"), Some(1_000));
    requests.refresh_native("native:1", question());
    assert_eq!(requests.views(9_000), before);
    assert_eq!(requests.deadline("native:1"), Some(1_000 + HOLD));
    // And its hold ends it at that deadline, marked or not.
    requests.refresh_native("native:1", marked());
    assert_eq!(requests.views(1_000 + HOLD - 1).len(), 1);
    assert!(requests.views(1_000 + HOLD).is_empty());
    assert!(dismissed(&requests, "native:1", 1_000 + HOLD));
}

/// An answer that could not be sent: the request is back as it was, where
/// it stood among the others, and marked.
#[test]
fn a_put_back_restores_the_request_as_it_was() {
    let requests = Requests::default();
    let first = requests
        .register_native("native:1".into(), question(), 1_000)
        .unwrap();
    requests
        .register_native("native:2".into(), question(), 2_000)
        .unwrap();
    requests.answer("native:1", &pick(), 3_000).unwrap();
    assert_eq!(first.response.recv().unwrap(), Some(pick()));
    assert_eq!(requests.views(3_000).len(), 1);

    let back = requests
        .put_back("native:1", first.wait, marked(), 60_000)
        .expect("the request still waits");
    assert_eq!(back.id, "native:1");
    assert_eq!(back.deadline_ms, 1_000 + HOLD);
    assert_ne!(back.wait, first.wait);
    let views = requests.views(60_000);
    let ids: Vec<&str> = views.iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["native:1", "native:2"]);
    assert_eq!(views[0]["unreachable"], WHY);
    assert_eq!(requests.deadline("native:1"), Some(1_000 + HOLD));
    assert_eq!(requests.waiting_at("client", "s"), Some(1_000));
    // Not dismissed: it is there, and it is not there twice.
    assert!(requests
        .register_native("native:1".into(), question(), 60_000)
        .is_none());
    assert_eq!(requests.views(60_000).len(), 2);
    // Marked, it takes no answer; unmarked, the new ticket hears it.
    assert!(requests.answer("native:1", &pick(), 61_000).is_err());
    requests.refresh_native("native:1", question());
    requests.answer("native:1", &pick(), 62_000).unwrap();
    assert_eq!(back.response.recv().unwrap(), Some(pick()));
    // The hold was never started anew.
    let again = requests.put_back("native:1", back.wait, marked(), 63_000);
    assert_eq!(again.unwrap().deadline_ms, 1_000 + HOLD);
}

/// The put-back call itself, asked to restore a request that has since
/// expired, been settled, been cancelled or been cleared: nothing comes
/// back, each time.
#[test]
fn a_put_back_is_refused_for_a_request_that_was_withdrawn() {
    type Ending = fn(&Requests) -> i64;
    let endings: [(&str, Ending, bool); 5] = [
        ("its hold ran out", |_| 1_000 + HOLD, true),
        (
            "its source settled it",
            |requests| {
                requests.settle_native("native:1");
                5_000
            },
            false,
        ),
        (
            "its source cancelled it",
            |requests| {
                requests.cancel("native:1");
                5_000
            },
            true,
        ),
        (
            "everything was cleared",
            |requests| {
                requests.clear();
                5_000
            },
            false,
        ),
        (
            "the hold was shortened past it",
            |requests| {
                requests.set_hold_ms(1_000);
                5_000
            },
            true,
        ),
    ];
    for (what, end, stays_dismissed) in endings {
        let requests = Requests::default();
        let ticket = requests
            .register_native("native:1".into(), question(), 1_000)
            .unwrap();
        requests.answer("native:1", &pick(), 2_000).unwrap();
        let now = end(&requests);
        let revision = requests.revision();
        assert!(
            requests
                .put_back("native:1", ticket.wait, marked(), now)
                .is_none(),
            "{what}"
        );
        assert!(requests.views(now).is_empty(), "{what}");
        assert_eq!(requests.deadline("native:1"), None, "{what}");
        assert_eq!(requests.revision(), revision, "{what}");
        assert_eq!(
            dismissed(&requests, "native:1", now),
            stays_dismissed,
            "{what}"
        );
    }
    // One moment before its hold runs out, it does come back.
    let requests = Requests::default();
    let ticket = requests
        .register_native("native:1".into(), question(), 1_000)
        .unwrap();
    requests.answer("native:1", &pick(), 2_000).unwrap();
    let last = 1_000 + HOLD - 1;
    assert!(requests
        .put_back("native:1", ticket.wait, marked(), last)
        .is_some());
    // A request that never was, and one that still waits, are not put back.
    assert!(requests.put_back("native:9", 1, marked(), last).is_none());
    assert!(requests
        .put_back("native:1", ticket.wait, marked(), last)
        .is_none());
    assert_eq!(requests.views(last).len(), 1);
}

/// A put-back names the wait its decision was taken in. One that comes
/// late, for a wait before the one a later decision was taken in, restores
/// nothing: the later decision stands.
#[test]
fn a_put_back_never_reverses_a_later_decision() {
    let requests = Requests::default();
    let first = requests
        .register_native("native:1".into(), question(), 1_000)
        .unwrap();
    requests.answer("native:1", &pick(), 2_000).unwrap();
    let second = requests
        .put_back("native:1", first.wait, question(), 3_000)
        .unwrap();
    // Answered again, in its second wait.
    requests.answer("native:1", &pick(), 4_000).unwrap();
    assert_eq!(second.response.recv().unwrap(), Some(pick()));
    // The first wait's failure arrives only now.
    assert!(requests
        .put_back("native:1", first.wait, marked(), 5_000)
        .is_none());
    assert!(requests.views(5_000).is_empty());
    assert!(dismissed(&requests, "native:1", 5_000));
    // The second wait's own can still be given back.
    let _back = requests
        .put_back("native:1", second.wait, marked(), 6_000)
        .expect("the later decision's own put-back");
    // A request settled and asked again under the same id is a new one:
    // what was taken from the old one is given back to nobody.
    requests
        .answer("native:1", &json!({"kind": "leave"}), 6_500)
        .unwrap();
    requests.settle_native("native:1");
    let third = requests
        .register_native("native:1".into(), question(), 7_000)
        .unwrap();
    assert!(requests
        .put_back("native:1", second.wait, marked(), 7_500)
        .is_none());
    assert_eq!(requests.deadline("native:1"), Some(third.deadline_ms));
    assert_eq!(third.deadline_ms, 7_000 + HOLD);
}

/// The requests of a client's hooks are answered as before: they have no
/// mark, and nothing of theirs is kept to give back.
#[test]
fn a_hook_request_is_never_put_back() {
    let requests = Requests::default();
    let request: agent_hud_agents::Request = serde_json::from_value(json!({
        "source": "claude", "sessionId": "s", "kind": "permission", "toolName": "Bash",
        "requiresUserReviewer": false, "summary": "ls", "suggestions": [], "questions": []
    }))
    .unwrap_or_else(|error| panic!("{error}"));
    let ticket = requests.register(request, 1_000);
    requests
        .answer(&ticket.id, &json!({"kind": "allow"}), 2_000)
        .unwrap();
    assert!(ticket.response.recv().unwrap().is_some());
    assert!(requests
        .put_back(&ticket.id, ticket.wait, marked(), 3_000)
        .is_none());
    assert!(requests.views(3_000).is_empty());
}
