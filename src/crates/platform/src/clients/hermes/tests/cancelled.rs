//! An answer that must not leave: its card closed, or the observer
//! stopped, while the answer was on its way. The pretend gateway holds a
//! reply back, so the moment between two steps of an answer is long enough
//! to change things in. The verifier's tests (`verifier.rs`) cover a
//! question; these cover an approval, a set of questions, and a retry.
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

/// Holds back the reply to the next `method` the gateway receives.
fn hold(gateway: &Fake, method: &'static str) -> Arc<Hold> {
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some((method, held.clone()));
    held
}

#[test]
fn an_approval_is_not_sent_once_hermes_settled_it() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "approval-answered-once", "approval.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "session.resume");
    let allow = json!({"kind": "allow"});
    lab.requests
        .answer(&id, &allow, crate::hooks::now_ms())
        .unwrap();
    until("the answer is subscribing", || {
        held.entered.load(Ordering::Acquire)
    });
    // Someone approves it elsewhere while the answer is still on its way.
    gateway.play("approval-answered-once", Some("approval.settled"));
    until("the card closes", || lab.log().contains("why=Settled"));
    held.released.store(true, Ordering::Release);
    until("the visit ends", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    assert!(gateway.world().calls("approval.respond").is_empty());
    assert!(!lab.log().contains("answer sent"), "{}", lab.log());
    // Nothing comes back for a request that is over.
    assert!(lab.cards().is_empty());
}

#[test]
fn an_approval_is_not_sent_once_the_observer_stopped() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "approval-answered-once", "approval.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "session.resume");
    let allow = json!({"kind": "allow"});
    lab.requests
        .answer(&id, &allow, crate::hooks::now_ms())
        .unwrap();
    until("the answer is subscribing", || {
        held.entered.load(Ordering::Acquire)
    });
    // Client hooks are turned off: the approval is no longer Agent HUD's
    // to give.
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    until("the visit ends", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    assert!(gateway.world().calls("approval.respond").is_empty());
}

#[test]
fn the_answers_to_a_set_are_not_written_once_the_set_is_over() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-batch-answered-here", "clarify");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "session.resume");
    let decision = json!({"kind": "answer", "answers": {"q0": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the answer is subscribing", || {
        held.entered.load(Ordering::Acquire)
    });
    // Hermes withdraws the set (answered elsewhere) before the frame goes.
    gateway.play("question-batch-answered-here", Some("request.cancel"));
    until("the card closes", || lab.log().contains("close id="));
    held.released.store(true, Ordering::Release);
    until("the visit ends", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    let result = |(_, frame): &(usize, Value)| frame["id"] == "srq-be9ca8d8cb91";
    assert!(!gateway.world().received.iter().any(result));
    assert!(!lab.log().contains("answer sent"), "{}", lab.log());
}

#[test]
fn an_answer_is_not_tried_again_once_the_observer_stopped() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    gateway.world().answers_with = Some("permission_denied");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "clarify.respond");
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the first try is refused", || {
        held.entered.load(Ordering::Acquire)
    });
    let socket = gateway.world().calls("clarify.respond")[0].0;
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    until("the visit ends", || {
        let parted = gateway.world().calls("session.detach");
        parted.iter().any(|(on, _)| *on == socket)
    });
    let world = gateway.world();
    // One try, and no second subscription for another.
    assert_eq!(world.calls("clarify.respond").len(), 1);
    let resumed = world.calls("session.resume");
    assert_eq!(resumed.iter().filter(|(on, _)| *on == socket).count(), 1);
}

#[test]
fn an_answer_still_goes_when_nothing_changed_while_it_was_on_its_way() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "session.resume");
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the answer is subscribing", || {
        held.entered.load(Ordering::Acquire)
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    held.released.store(true, Ordering::Release);
    until("the answer lands", || {
        lab.delivery(&id)["state"] == "confirmed"
    });
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
}

#[test]
fn an_answer_keeps_its_receipt_when_a_poll_closes_the_card_before_hermes_replies() {
    // Hermes takes the answer and settles the request, and its reply is
    // slow: the poll sees the request settled and closes the card first.
    // The reply, when it comes, is all that says what became of the answer.
    for (reply, state) in [(None, "confirmed"), (Some("already_resolved"), "failed")] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        gateway.world().answers_with = reply;
        lab.follow();
        lab.hears(&gateway, "question-answered-here", "clarify.request");
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let held = hold(&gateway, "clarify.respond");
        let decision =
            json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
        lab.requests
            .answer(&id, &decision, crate::hooks::now_ms())
            .unwrap();
        until("Hermes holds its reply", || {
            held.entered.load(Ordering::Acquire)
        });
        until("a poll closes the card", || {
            lab.log().contains("why=Settled")
        });
        assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
        held.released.store(true, Ordering::Release);
        assert_eq!(lab.delivery(&id)["state"], state, "{reply:?}");
        // The card stays closed, and nothing is said twice.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(lab.cards().is_empty());
        assert_eq!(lab.requests.deliveries(crate::hooks::now_ms()).len(), 1);
        assert!(!lab.log().contains("unanswered id="), "{}", lab.log());
    }
}
