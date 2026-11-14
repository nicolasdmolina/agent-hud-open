//! The second review of ticket 27 (`evidence/27/verifier-report-2.md`):
//! the verifier's five further probes (`reviewer2-extra.rs`), tracked as
//! tests. Every assertion stands. Besides the formatting, three lines
//! are changed, all in the first probe: it set and cleared a gate that
//! was one static for the whole process, named by the request's id. Tests
//! that run side by side replay the same recordings and so share request
//! ids, and another test's answer waited at that gate (the verifier ran
//! its probes one at a time). The gate is now the lab's own requests'
//! (`Requests::reviewer27_hold_acceptance`), and so are the receipts the
//! probes read.
//!
//! One more line is changed, in the cut-writer probe: its connection
//! gives a frame a minute to get out, where the tests' usual pace gives
//! three seconds. The probe keeps the gateway from reading the new
//! answer while it lets the old writer return, and the new answer has to
//! be still on its way when the gateway reads again. On a machine at
//! load 70 to 155 those steps took over three seconds: the new answer
//! gave up and its card came back, with its record kept, which is what
//! the probe is about, and the probe then waited for an answer that was
//! not coming (4 of 30 runs of the whole suite, and 12 of 36 at load
//! 200). Nothing the probe asserts depends on that time.
//!
//! Two of the five failed on `4525903`:
//!
//! - `…acceptance_before_mark_publication_after_reconnect_needs_a_new_click`
//!   holds core between taking a decision and handing it over, marks the
//!   card, reconnects and
//!   reconciles, and only then lets the decision through. It must not be
//!   sent.
//! - `…old_cut_writer_must_not_remove_the_fresh_answers_receipt` lets a
//!   cut writer return while a new answer to the same card is being
//!   written. The new answer must keep its receipt.
//!
//! The other three passed there and are controls: a marked click is
//! refused and a fresh one goes through, a second loss, and two cards in
//! one session. They read every receipt core was told of, in order
//! (`Requests::reviewer27_notes`), so one receipt replacing another in
//! core's rows cannot hide a missing one.
use super::unreachable::{asks, follower, red, started};
use super::{fake::Hold, lab, marked, until};
use agent_hud_agents::hermes::gateway::{Failure, Input};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn now() -> i64 {
    crate::hooks::now_ms()
}
fn notes(lab: &super::Lab, id: &str) -> Vec<String> {
    lab.requests.reviewer27_notes(id)
}

#[test]
fn reviewer27_extra_acceptance_before_mark_publication_after_reconnect_needs_a_new_click() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let gate = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    lab.requests
        .reviewer27_hold_acceptance(Some((id.clone(), gate.clone())));
    let requests = lab.requests.clone();
    let request = id.clone();
    let accepted = std::thread::spawn(move || requests.answer(&request, &red(), now()));
    until("core removed card but has not published decision", || {
        gate.0.load(Ordering::Acquire)
    });
    assert!(lab.cards().is_empty());
    let identity = gateway.world().identify.clone().unwrap();
    {
        let mut model = shared.model();
        shared.take(&mut model, Input::Lost(Failure::Closed(1006)));
        eprintln!("TRACE accepted before mark, channel not sent: marked={} waiter={} taken={} core_cards={}", model.gateway.marked(&id), model.waiters.contains_key(&id), model.taken.contains_key(&id), lab.cards().len());
        shared.take(
            &mut model,
            Input::Identified {
                answer: &identity,
                home: lab.home.to_str().unwrap(),
            },
        );
    }
    poll.released.store(true, Ordering::Release);
    shared.want_poll();
    until("accepted poll unmarks", || {
        !shared.model().gateway.marked(&id)
    });
    gate.1.store(true, Ordering::Release);
    accepted.join().unwrap().unwrap();
    lab.requests.reviewer27_hold_acceptance(None);
    until("old decision handled", || {
        !gateway.world().calls("clarify.respond").is_empty() || lab.cards().len() == 1
    });
    eprintln!(
        "TRACE no second click: received_answers={} core_cards={} notes={:?}\n{}",
        gateway.world().calls("clarify.respond").len(),
        lab.cards().len(),
        notes(&lab, &id),
        lab.log()
    );
    let count = gateway.world().calls("clarify.respond").len();
    assert_eq!(
        count, 0,
        "pre-mark decision survived the unpublished channel handoff"
    );
    assert_eq!(lab.cards().len(), 1);
}

#[test]
fn reviewer27_extra_marked_click_refused_fresh_reconnect_click_preserved() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    lab.route.shut.store(true, Ordering::Release);
    shared.apply(Input::Lost(Failure::Unreachable));
    let before = lab.cards();
    let revision = lab.requests.revision();
    assert!(lab.requests.answer(&id, &red(), now()).is_err());
    assert_eq!(lab.cards(), before);
    assert_eq!(lab.requests.revision(), revision);
    assert!(notes(&lab, &id).is_empty());
    assert!(gateway.world().calls("clarify.respond").is_empty());
    lab.route.shut.store(false, Ordering::Release);
    until("card answerable again", || !marked(&lab.card()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("fresh answer confirmed", || {
        notes(&lab, &id) == ["confirmed"]
    });
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
    assert_eq!(lab.requests.deliveries(now()).len(), 1);
    eprintln!(
        "TRACE marked click refused; post-reconnect answer sent once; receipts={:?}",
        notes(&lab, &id)
    );
}

#[test]
fn reviewer27_extra_second_loss_cuts_the_new_wait_and_preserves_the_next_click() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let first = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.resume", first.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("first subscription held", || {
        first.entered.load(Ordering::Acquire)
    });
    let identity = gateway.world().identify.clone().unwrap();
    {
        let mut m = shared.model();
        shared.take(&mut m, Input::Lost(Failure::Closed(1006)));
        shared.take(
            &mut m,
            Input::Identified {
                answer: &identity,
                home: lab.home.to_str().unwrap(),
            },
        );
    }
    poll.released.store(true, Ordering::Release);
    shared.want_poll();
    until("first mark clears", || !marked(&lab.card()));
    let second = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", second.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("fresh second decision in prepare", || {
        second.entered.load(Ordering::Acquire)
    });
    lab.route.shut.store(true, Ordering::Release);
    shared.apply(Input::Lost(Failure::Unreachable));
    assert!(marked(&lab.card()));
    second.released.store(true, Ordering::Release);
    first.released.store(true, Ordering::Release);
    until("both failed decisions have receipts", || {
        notes(&lab, &id).len() == 2
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert_eq!(notes(&lab, &id), ["failed", "failed"]);
    lab.route.shut.store(false, Ordering::Release);
    until("second mark clears", || !marked(&lab.card()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("third click confirmed", || notes(&lab, &id).len() == 3);
    assert_eq!(notes(&lab, &id), ["failed", "failed", "confirmed"]);
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
    assert_eq!(lab.requests.deliveries(now()).len(), 1);
    eprintln!(
        "TRACE two losses, zero stale sends, fresh third answer once; receipts={:?}",
        notes(&lab, &id)
    );
}

#[test]
fn reviewer27_extra_old_cut_writer_must_not_remove_the_fresh_answers_receipt() {
    use super::super::{answer, visit::Visit};
    use serde_json::json;
    use std::time::Duration;
    let mut lab = lab();
    // A minute for a frame to get out, where the tests' pace gives three
    // seconds (see the top of this file).
    let patient = super::super::Pace {
        write: Duration::from_secs(60),
        ..super::quick()
    };
    let gateway = super::unreachable::started_at(&mut lab, patient);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    // Take exactly the real core decision and its real wait, before the watcher can.
    let (pending, instance, old_wait) = {
        let mut model = shared.model();
        let waiter = model.waiters.remove(&id).unwrap();
        lab.requests.answer(&id, &red(), now()).unwrap();
        assert!(waiter.response.recv().unwrap().is_some());
        model.taken.insert(id.clone(), waiter.wait);
        (
            model.gateway.pending(&id).unwrap().clone(),
            model.instance_of(&id).unwrap(),
            waiter.wait,
        )
    };
    let agent_hud_agents::hermes::gateway::Reply::Call(command) = pending
        .reply(&red(), agent_hud_agents::hermes::gateway::Scope::Host)
        .unwrap()
        .unwrap()
    else {
        panic!("expected question call")
    };
    let old = Arc::new(Hold::default());
    std::thread::scope(|scope| {
        let old_writer = scope.spawn(|| {
            let (mut visit, _) = Visit::begin_for(
                &shared,
                &instance,
                &pending.session,
                &id,
                Duration::from_secs(2),
            )
            .unwrap();
            let written: Option<u64> = answer::verifier27_written_with_wait(
                &shared,
                &pending,
                &mut visit,
                old_wait,
                |socket| {
                    assert!(shared.model.try_lock().is_ok());
                    old.entered.store(true, Ordering::Release);
                    until("old cut writer returns", || {
                        old.released.load(Ordering::Acquire)
                    });
                    socket.send(&command)
                },
            );
            assert!(written.is_none());
            visit.end();
        });
        until("first writer admitted", || {
            old.entered.load(Ordering::Acquire)
        });
        let identity = gateway.world().identify.clone().unwrap();
        {
            let mut m = shared.model();
            shared.take(&mut m, Input::Lost(Failure::Closed(1006)));
            shared.take(
                &mut m,
                Input::Identified {
                    answer: &identity,
                    home: lab.home.to_str().unwrap(),
                },
            );
        }
        poll.released.store(true, Ordering::Release);
        shared.want_poll();
        until("mark clears", || !marked(&lab.card()));
        let stall = Arc::new(Hold::default());
        gateway.world().stall_after_resume = Some(stall.clone());
        let fresh =
            json!({"kind":"answer","answers":{"e1bc9d4fa6":{"custom":"x".repeat(8*1024*1024)}}});
        lab.requests.answer(&id, &fresh, now()).unwrap();
        until("fresh peer stops reading", || {
            stall.entered.load(Ordering::Acquire)
        });
        until("fresh writer admitted alongside old cut writer", || {
            shared.reviewer27_writing_count() == 2
        });
        {
            let m = shared.model();
            assert_ne!(m.taken.get(&id), Some(&old_wait));
            assert!(m.answered.contains_key(&id));
        }
        let fresh_socket = gateway.world().calls("session.resume").last().unwrap().0;
        old.released.store(true, Ordering::Release);
        old_writer.join().unwrap();
        eprintln!(
            "TRACE old cut writer returned: fresh Out retained={} notes={:?}",
            shared.model().answered.contains_key(&id),
            notes(&lab, &id)
        );
        stall.released.store(true, Ordering::Release);
        until("fresh full answer received", || {
            gateway.world().calls("clarify.respond").len() == 1
        });
        until("fresh answer closes card", || {
            shared.model().gateway.pending(&id).is_none()
        });
        until("fresh answer visit ended", || {
            gateway.world().ended.contains(&fresh_socket)
        });
        eprintln!(
            "TRACE fresh answer accepted by gateway: answers={} notes={:?} deliveries={:?}\n{}",
            gateway.world().calls("clarify.respond").len(),
            notes(&lab, &id),
            lab.requests.deliveries(now()),
            lab.log()
        );
        assert_eq!(
            lab.delivery(&id)["state"],
            "confirmed",
            "stale writer cleanup removed the fresh answer receipt"
        );
        assert_eq!(notes(&lab, &id), ["uncertain", "confirmed"]);
    });
}

#[test]
fn reviewer27_extra_two_cards_in_one_session_are_cut_and_restored_independently() {
    use serde_json::json;
    let mut lab = lab();
    let gateway = started(&mut lab);
    let first = asks(&lab, &gateway);
    let shared = follower(&lab);
    {
        let mut w = gateway.world();
        let stream = w.streams.values_mut().next().unwrap();
        let mut event = stream
            .events
            .iter()
            .find(|e| e["type"] == "clarify.request")
            .unwrap()
            .clone();
        event["payload"]["prompt_id"] = json!("reviewer-second");
        stream.latest += 1;
        event["seq"] = json!(stream.latest);
        stream.prompts.push(event["payload"].clone());
        stream.events.push(event);
    }
    gateway.say("sessions.changed");
    until("two cards same session", || lab.cards().len() == 2);
    let cards = lab.cards();
    assert_eq!(cards[0]["sessionId"], cards[1]["sessionId"]);
    let second = cards.iter().find(|c| c["id"] != first).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    lab.route.shut.store(true, Ordering::Release);
    {
        let mut model = shared.model();
        lab.requests.answer(&first, &red(), now()).unwrap();
        lab.requests
            .answer(
                &second,
                &json!({"kind":"answer","answers":{"reviewer-second":{"selected":["red"]}}}),
                now(),
            )
            .unwrap();
        shared.take(&mut model, Input::Lost(Failure::Unreachable));
    }
    assert_eq!(lab.cards().len(), 2);
    assert!(lab.cards().iter().all(marked));
    assert!(lab
        .requests
        .answer(
            &second,
            &json!({"kind":"answer","answers":{"reviewer-second":{"selected":["red"]}}}),
            now()
        )
        .is_err());
    assert_eq!(notes(&lab, &first), ["failed"]);
    assert_eq!(notes(&lab, &second), ["failed"]);
    lab.route.shut.store(false, Ordering::Release);
    until("both marks clear", || {
        lab.cards().len() == 2 && lab.cards().iter().all(|c| !marked(c))
    });
    lab.requests
        .answer(
            &second,
            &json!({"kind":"answer","answers":{"reviewer-second":{"selected":["red"]}}}),
            now(),
        )
        .unwrap();
    until("second card confirmed", || {
        notes(&lab, &second) == ["failed", "confirmed"]
    });
    assert_eq!(lab.cards().len(), 1);
    assert_eq!(lab.cards()[0]["id"], first);
    lab.requests.answer(&first, &red(), now()).unwrap();
    until("first card confirmed", || {
        notes(&lab, &first) == ["failed", "confirmed"]
    });
    assert_eq!(gateway.world().calls("clarify.respond").len(), 2);
    assert_eq!(lab.requests.deliveries(now()).len(), 2);
    eprintln!("TRACE two cards one session, two fresh answers, independent receipts: first={:?} second={:?}",notes(&lab,&first),notes(&lab,&second));
}
