//! One receipt for every answer core took, whichever way it ends (ticket
//! 27, the fourth review). The reviewer's probes are in
//! `reviewer27_round4.rs`; these are two more ends they did not reach.
//! Receipts are counted as core was told of them, in order, before a
//! later one for the same request replaces the row.
use super::unreachable::{asks, follower, red, started, ASKED};
use super::{fake::Hold, lab, until};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

fn now() -> i64 {
    crate::hooks::now_ms()
}

/// Core took the decision and handed it over, and the answers' thread has
/// not picked it up when a poll finds the request settled. The card
/// closes, and the decision ends with it: it failed, said once. Nothing
/// is sent when the thread looks again.
#[test]
fn a_decision_that_waits_to_be_picked_up_when_its_card_closes_has_one_receipt() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    shared.deaf.store(true, Ordering::Release);
    lab.requests.answer(&id, &red(), now()).unwrap();
    {
        let model = shared.model();
        assert!(model.waiters.contains_key(&id) && !model.taken.contains_key(&id));
    }
    gateway.play(ASKED, Some("clarify.settled"));
    shared.want_poll();
    until("the poll closes the card", || {
        lab.log().contains(&format!("close id={id} why=Settled"))
    });
    // Waited for on the receipt itself: the log says the close a moment
    // before the receipt is told.
    until("the decision has its receipt", || {
        !lab.requests.reviewer27_notes(&id).is_empty()
    });
    assert_eq!(lab.requests.reviewer27_notes(&id), ["failed"]);
    shared.deaf.store(false, Ordering::Release);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(lab.requests.reviewer27_notes(&id), ["failed"]);
    assert!(lab.cards().is_empty());
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert!(gateway.world().calls("session.resume").is_empty());
    let model = shared.model();
    assert!(!model.taken.contains_key(&id) && !model.answered.contains_key(&id));
}

/// The answer to a set of questions is written, and nothing replies to
/// it. Then its session is gone from the gateway's list before a poll
/// could see the set taken. Whether Hermes took the answer first, nothing
/// says: the receipt is uncertain, and there is one. Before, a set that
/// ended any other way than settled had none.
#[test]
fn a_set_whose_session_ends_after_its_answer_was_written_has_one_receipt() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    lab.hears(&gateway, "question-batch-answered-here", "clarify");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("a poll is held", || poll.entered.load(Ordering::Acquire));
    let decision = json!({"kind": "answer", "answers": {
        "q0": {"selected": ["green"]},
        "q1": {"selected": ["small (Recommended)"]},
    }});
    lab.requests.answer(&id, &decision, now()).unwrap();
    let result = |(_, frame): &(usize, Value)| frame["id"] == "srq-be9ca8d8cb91";
    until("Hermes hears the answers", || {
        gateway.world().received.iter().any(result)
    });
    until("the answer is told as written", || {
        lab.log().contains("answer sent id=")
    });
    assert!(lab.requests.reviewer27_notes(&id).is_empty());
    // The session is over, and the gateway lists it no more.
    gateway.world().streams.clear();
    poll.released.store(true, Ordering::Release);
    shared.want_poll();
    until("the card closes", || lab.log().contains("close id="));
    until("the answer has its receipt", || {
        !lab.requests.reviewer27_notes(&id).is_empty()
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(lab.requests.reviewer27_notes(&id), ["uncertain"]);
    assert!(!lab.log().contains("why=Settled"), "{}", lab.log());
    assert!(lab.cards().is_empty());
}

/// Turning client hooks off leaves no receipt, and the stop itself is no
/// lost connection. A follower says it is ending before it takes its
/// model to end, and its own thread, seeing that, drops the poll it was
/// in. That used to be read as a lost socket in the moment between: the
/// card was marked, and an answer that was out got a receipt that said
/// it failed (once in 22 runs of the suite, at load 70).
#[test]
fn a_follower_that_is_ending_reads_no_lost_connection_into_its_stop() {
    use agent_hud_agents::hermes::gateway::{Failure, Input};
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    // The answer is out, and held back before it opens anything.
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the send is held back", || {
        held.entered.load(Ordering::Acquire)
    });
    assert!(shared.model().taken.contains_key(&id));
    // The follower has said it is ending, and has not taken its model yet.
    shared.active.store(false, Ordering::Release);
    shared.apply(Input::Lost(Failure::Unreachable));
    {
        let model = shared.model();
        assert!(!model.gateway.marked(&id), "the stop was read as a loss");
        assert!(model.taken.contains_key(&id));
    }
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(200));
    assert!(lab.requests.reviewer27_notes(&id).is_empty());
    assert!(lab.cards().is_empty());
    assert!(!lab.log().contains("undelivered"), "{}", lab.log());
    assert!(!lab.log().contains("unreachable id="), "{}", lab.log());
    assert!(gateway.world().calls("clarify.respond").is_empty());
}
