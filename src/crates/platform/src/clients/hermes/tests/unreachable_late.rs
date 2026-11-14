//! Answers and a connection that goes (ticket 27). Core takes a decision
//! and puts the card away before the adapter has sent anything. Where the
//! connection goes in between, the adapter sends nothing and gives the
//! request back as it was: the same id, the time it came, its hold, not
//! dismissed, marked, without a sound, and never registered anew. Where
//! the request came to its end meanwhile (its hold ran out, Hermes
//! settled it, client hooks turned off), the failure that comes late
//! restores nothing.
//!
//! The order is forced, not left to timing. For the race, the test holds
//! the follower's model while core takes the decision and while the
//! connection is lost, so the answers' thread sees both only afterwards.
//! For the late failures, the send is held back on its way (at the route,
//! or at the gateway's reply) until what ends the request has happened.
use super::unreachable::{
    asks, dismissed, follower, needs, red, shows, snapshot, sounding, started, times, ASKED,
};
use super::{
    fake::{Hold, World},
    lab, marked, plain, until,
};
use agent_hud_agents::hermes::gateway::{out_of_reach, Failure, Input};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

fn now() -> i64 {
    crate::hooks::now_ms()
}

/// Core accepted the decision, and the connection is dropped before the
/// adapter's send. `sent` says whether the gateway got an answer.
fn loses_the_race(recording: &str, through: &str, decision: Value, sent: fn(&World) -> bool) {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let gateway = started(&mut lab);
    snapshot(&mut core);
    lab.hears(&gateway, recording, through);
    let card = lab.card();
    let id = card["id"].as_str().unwrap().to_owned();
    assert_eq!(needs(&mut core), 1, "the request sounds once");
    let before = times(&lab, &card);
    let (shared, registered) = (follower(&lab), shows(&lab));
    // From here nobody says who is there: a follower that loses its
    // socket finds no way back. The gateway itself stays up, and would
    // give a ticket, take a socket and take an answer.
    let told = gateway.world().identify.take();
    let (sockets, tickets) = {
        // The answers' thread and the polls wait for the model here.
        let mut model = shared.model();
        lab.requests.answer(&id, &decision, now()).unwrap();
        assert!(lab.cards().is_empty(), "core put the card away");
        let world = gateway.world();
        let open = (world.sockets, world.granted.len());
        drop(world);
        // The connection goes, as the follower's own thread says it.
        gateway.cut();
        shared.take(&mut model, Input::Lost(Failure::Closed(1006)));
        open
    };

    // The card is back: the same request, as it was, and marked.
    let back = lab.card();
    assert_eq!(back["id"], id.as_str());
    assert_eq!(back["createdAt"], card["createdAt"]);
    assert_eq!(back["unreachable"], json!(out_of_reach()));
    assert_eq!(plain(back), card);
    assert_eq!(times(&lab, &card), before, "its time and its hold");
    assert_eq!(shows(&lab), registered, "register_native was not called");
    assert_eq!(needs(&mut core), 1, "no alert");
    assert_eq!(lab.delivery(&id)["state"], "failed");
    assert!(lab.log().contains("undelivered id="), "{}", lab.log());
    // Nothing was sent: no ticket, no socket, no answer.
    std::thread::sleep(Duration::from_millis(200));
    let world = gateway.world();
    assert!(!sent(&world), "an answer went through a lost connection");
    assert_eq!(world.sockets, sockets, "no socket was opened");
    assert_eq!(world.granted.len(), tickets, "no ticket was asked for");
    assert!(world.calls("session.resume").is_empty());
    drop(world);
    assert!(marked(&lab.card()), "marked while it is out of reach");

    // The gateway is found again: the mark goes, and the same card, with
    // the hold it always had, takes the answer.
    gateway.world().identify = told;
    until("the mark clears", || !marked(&lab.card()));
    assert_eq!(lab.card(), card);
    assert_eq!(times(&lab, &card), before);
    assert_eq!(needs(&mut core), 1, "no alert");
    lab.requests.answer(&id, &decision, now()).unwrap();
    until("Hermes hears the answer", || sent(&gateway.world()));
    // The note of the first try stands until this one has its receipt.
    until("the answer is confirmed", || {
        lab.delivery(&id)["state"] == "confirmed"
    });
    assert!(lab.cards().is_empty());
    assert_eq!(shows(&lab), registered);
}

#[test]
fn an_answer_that_loses_the_race_is_put_back() {
    let answered = |world: &World| !world.calls("clarify.respond").is_empty();
    loses_the_race(ASKED, "clarify.request", red(), answered);
}

/// The answer to a set of questions is a result frame that nothing
/// replies to: there is no reply to wait for, and none to miss.
#[test]
fn the_same_for_a_set_of_questions() {
    let decision = json!({"kind": "answer", "answers": {
        "q0": {"selected": ["green"]},
        "q1": {"selected": ["small (Recommended)", "large"]},
    }});
    let answered = |world: &World| {
        let result = |(_, frame): &(usize, Value)| frame["id"] == "srq-be9ca8d8cb91";
        world.received.iter().any(result)
    };
    loses_the_race(
        "question-batch-answered-here",
        "clarify",
        decision,
        answered,
    );
}

#[test]
fn a_late_failure_after_the_hold_expired_restores_nothing() {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let mut gateway = started(&mut lab);
    snapshot(&mut core);
    let id = asks(&lab, &gateway);
    let card = lab.card();
    assert_eq!(needs(&mut core), 1);
    let (_, deadline) = times(&lab, &card);
    let registered = shows(&lab);
    // The decision is accepted, and its send is held back on the way.
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the send is held back", || {
        held.entered.load(Ordering::Acquire)
    });
    // The card's original hold runs out.
    lab.requests.expire(deadline);
    // Then the send fails: the gateway is gone when it is let go.
    gateway.stop();
    held.released.store(true, Ordering::Release);
    until("the failure returns", || lab.log().contains("over id="));

    assert!(lab.cards().is_empty(), "no card comes back");
    assert!(dismissed(&lab, &card), "it still counts as dismissed");
    assert_eq!(needs(&mut core), 1, "no alert");
    assert_eq!(shows(&lab), registered);
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty(), "nothing is sent");
    assert!(world.calls("session.resume").is_empty());
    drop(world);
    // And it stays away while the follower goes on asking.
    std::thread::sleep(Duration::from_millis(200));
    assert!(lab.cards().is_empty());
    assert_eq!(lab.requests.deadline(&id), None);
}

#[test]
fn a_late_failure_after_hermes_settled_restores_nothing() {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let gateway = started(&mut lab);
    snapshot(&mut core);
    let id = asks(&lab, &gateway);
    assert_eq!(needs(&mut core), 1);
    let registered = shows(&lab);
    // The decision is accepted, and its send is held back: the gateway
    // keeps its reply to the subscription the answer needs.
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.resume", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the send is held back", || {
        held.entered.load(Ordering::Acquire)
    });
    // A poll sees the request settled: it was answered elsewhere.
    gateway.play(ASKED, Some("clarify.settled"));
    until("the poll closes the card", || {
        lab.log().contains(&format!("close id={id} why=Settled"))
    });
    // Only now does the send fail: its socket is cut.
    gateway.cut();
    held.released.store(true, Ordering::Release);
    until("the gateway is done with the answer's socket", || {
        gateway.world().visit_over()
    });
    std::thread::sleep(Duration::from_millis(200));

    assert!(lab.cards().is_empty(), "no card comes back");
    assert_eq!(lab.requests.deadline(&id), None);
    assert_eq!(needs(&mut core), 1, "no alert");
    assert_eq!(shows(&lab), registered);
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert!(lab.requests.deliveries(now()).is_empty());
    assert!(!lab.log().contains("undelivered"), "{}", lab.log());
}

#[test]
fn a_late_failure_after_hooks_turned_off_restores_nothing() {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let mut gateway = started(&mut lab);
    snapshot(&mut core);
    let id = asks(&lab, &gateway);
    assert_eq!(needs(&mut core), 1);
    let shared = follower(&lab);
    let registered = shows(&lab);
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the send is held back", || {
        held.entered.load(Ordering::Acquire)
    });
    // Client hooks are turned off: every card is withdrawn.
    lab.observer.take();
    assert!(lab.cards().is_empty());
    // Only now does the send fail.
    gateway.stop();
    held.released.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(300));

    assert!(lab.cards().is_empty(), "no card comes back");
    assert_eq!(lab.requests.deadline(&id), None);
    assert_eq!(needs(&mut core), 1, "no alert");
    assert_eq!(shared.shows.load(Ordering::Acquire), registered);
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty(), "nothing is sent");
    assert!(world.calls("session.resume").is_empty());
    drop(world);
    assert!(lab.requests.deliveries(now()).is_empty());
    assert!(!lab.log().contains("undelivered"), "{}", lab.log());
}

/// A put-back never reverses a later decision, at the adapter too. The
/// first answer is written, and Hermes's word on it comes late: no reply
/// at all, or a refusal that an answer is tried again after. Meanwhile
/// the socket is lost under it, so the card comes back, and it is
/// answered again. What the first answer's thread then learns is of an
/// answer that is no longer the one that is out: it writes nothing more,
/// and it does not bring the card back a second time.
#[test]
fn a_failure_that_comes_late_for_an_earlier_answer_leaves_the_later_one_alone() {
    for late in ["silence", "permission_denied"] {
        late_for_an_earlier_answer(late);
    }
}

fn late_for_an_earlier_answer(late: &'static str) {
    let mut lab = lab();
    let gateway = super::fake::Fake::start(&lab.home, "instance-1");
    // A second of patience for a call, so the first answer gives up soon.
    let patient = super::super::Pace {
        answer: Duration::from_secs(1),
        ..super::quick()
    };
    lab.follow_at(patient);
    let id = asks(&lab, &gateway);
    let card = lab.card();
    let before = times(&lab, &card);
    let shared = follower(&lab);
    let answers = |gateway: &super::fake::Fake| gateway.world().calls("clarify.respond");

    // The first answer goes out. Hermes never replies to it, or its
    // refusal is kept back for now.
    let kept = Arc::new(Hold::default());
    {
        let mut world = gateway.world();
        world.answers_with = Some(late);
        if late != "silence" {
            world.hold_next = Some(("clarify.respond", kept.clone()));
        }
    }
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the first answer is written", || {
        answers(&gateway).len() == 1
    });
    let (first, _) = answers(&gateway)[0];
    // The socket is lost under it: the card comes back, as it was.
    shared.take(&mut shared.model(), Input::Lost(Failure::Closed(1006)));
    until("the card is back", || lab.cards().len() == 1);
    assert_eq!(times(&lab, &card), before);
    until("the mark clears", || !marked(&lab.card()));
    assert_eq!(lab.card(), card);

    // It is answered again, and this answer is held back on its way.
    gateway.world().answers_with = None;
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("the second answer is on its way", || {
        held.entered.load(Ordering::Acquire)
    });
    // The first answer's thread hears at last, or gives up waiting. It
    // would try once more after a refusal like this one, and then it is
    // done with its socket. All the while the card stays put away.
    kept.released.store(true, Ordering::Release);
    until("the first answer gives up", || {
        assert!(lab.cards().is_empty(), "the later decision was reversed");
        gateway.world().ended.contains(&first)
    });
    assert!(lab.cards().is_empty(), "the later decision was reversed");
    assert_eq!(answers(&gateway).len(), 1, "{late}: it was written again");
    let log = lab.log();
    assert_eq!(log.matches("unanswered id=").count(), 1, "{log}");
    // The later answer goes out, lands, and has the receipt.
    held.released.store(true, Ordering::Release);
    until("the second answer is confirmed", || {
        lab.delivery(&id)["state"] == "confirmed"
    });
    assert_eq!(answers(&gateway).len(), 2);
    assert!(lab.cards().is_empty());
    assert_eq!(shows(&lab), 1, "one request, registered once");
}
