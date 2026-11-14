//! The independent verifier's probes from the fourth review of ticket 13
//! (`evidence/13/verifier-4-extra.patch`), kept as written but for where
//! the first one pauses the answer's thread. It paused between the reply
//! handler looking at the card and applying the reply, which were two
//! holds of the model's lock; they are one now, so there is no such place.
//! It pauses right before that one hold, after the reply is read, which is
//! where a poll can still close the card (`answer::BEFORE_REPLY`, set by
//! the test). The pause is keyed by the observer as well as the request:
//! tests that run side by side replay the same recordings and so share
//! request ids, and keyed by the id alone another test's answer walked
//! into it. And the recorded settlement is played for a stale-generation
//! refusal in both orderings, as the fifth review's diagnostic did: the
//! pretend gateway refused the answer as stale and still listed its
//! request as waiting, so the poll let go at the end reopened the card
//! about once in thirty runs. Every assertion stands. The fifth review
//! found that the moved pause no longer tells a handler with two holds
//! from one with one; `replies.rs` checks that at the place itself. Only
//! pretend gateways and temporary homes.
use super::super::{answer, visit::Visit};
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use agent_hud_agents::hermes::gateway::{Reply, Scope};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::time::Duration;

static GAP: Mutex<Option<(usize, String, Arc<Hold>)>> = Mutex::new(None);

// Called only in test builds, with nothing locked, right before
// Going::replied handles Hermes's reply. It changes no production decision.
pub(crate) fn before_response_apply(observer: usize, id: &str) {
    let held = GAP
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(of, request, _)| *of == observer && request == id)
        .map(|(_, _, held)| held.clone());
    if let Some(held) = held {
        held.entered.store(true, Ordering::Release);
        until("reply check/apply probe released", || {
            held.released.load(Ordering::Acquire)
        });
    }
}

fn scenario(
    kind: &str,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Value,
) {
    match kind {
        "question" => (
            "question-answered-here",
            "clarify.request",
            "clarify.respond",
            "clarify.settled",
            json!({"kind":"answer","answers":{"715a2db63d":{"selected":["green"]}}}),
        ),
        "approval" => (
            "approval-answered-once",
            "approval.request",
            "approval.respond",
            "approval.settled",
            json!({"kind":"allow"}),
        ),
        "batch" => (
            "question-batch-answered-here",
            "clarify",
            "",
            "request.cancel",
            json!({"kind":"answer","answers":{"q0":{"selected":["green"]}}}),
        ),
        _ => unreachable!(),
    }
}

#[test]
fn verifier4_poll_between_reply_check_and_apply_must_keep_question_and_approval_receipts() {
    let mut observations = Vec::new();
    for kind in ["question", "approval"] {
        for reply in [None, Some("already_resolved"), Some("stale_generation")] {
            for preempt in [false, true] {
                let mut lab = lab();
                let gateway = Fake::start(&lab.home, "instance-1");
                gateway.world().answers_with = reply;
                lab.follow();
                let (recording, request, method, settled, decision) = scenario(kind);
                lab.hears(&gateway, recording, request);
                let id = lab.card()["id"].as_str().unwrap().to_owned();
                let shared = lab.observer.as_ref().unwrap().local();
                // Pause the real follower before submitting the answer so
                // the card cannot close before the first reply-state check.
                let poll = Arc::new(Hold::default());
                gateway.world().hold_next = Some(("session.active_list", poll.clone()));
                shared.want_poll();
                until("poll paused before answer", || {
                    poll.entered.load(Ordering::Acquire)
                });
                let gap = Arc::new(Hold::default());
                *answer::BEFORE_REPLY.write().unwrap() = Some(before_response_apply);
                let observer = Arc::as_ptr(&shared) as usize;
                *GAP.lock().unwrap() = Some((observer, id.clone(), gap.clone()));
                lab.requests
                    .answer(&id, &decision, crate::hooks::now_ms())
                    .unwrap();
                until("reply read and not yet handled, model unlocked", || {
                    gap.entered.load(Ordering::Acquire)
                });
                assert!(shared.model().gateway.pending(&id).is_some());
                assert!(shared.model().answered.get(&id).unwrap().closed.is_none());
                let socket = gateway.world().calls(method)[0].0;
                // Success and already_resolved settle in the fake gateway
                // before replying. A stale-generation refusal is followed
                // by the recorded external settlement, in both orderings:
                // a request refused as stale is over in the gateway too,
                // and a poll let go afterwards must not find it waiting.
                if reply == Some("stale_generation") {
                    gateway.play(recording, Some(settled));
                }
                if preempt {
                    poll.released.store(true, Ordering::Release);
                    shared.want_poll();
                    until("real poll closes card in reply check/apply gap", || {
                        shared.model().gateway.pending(&id).is_none()
                    });
                    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
                }
                gap.released.store(true, Ordering::Release);
                until("specific answer visit ended after applying reply", || {
                    gateway.world().ended.contains(&socket)
                });
                poll.released.store(true, Ordering::Release);
                *GAP.lock().unwrap() = None;
                let rows = lab.requests.deliveries(crate::hooks::now_ms());
                let expected = if reply.is_none() {
                    "confirmed"
                } else {
                    "failed"
                };
                let correct =
                    rows.len() == 1 && rows[0]["id"] == id && rows[0]["state"] == expected;
                let retained = shared.model().answered.contains_key(&id);
                eprintln!("REPLY GAP kind={kind} reply={reply:?} preempt={preempt} expected={expected} deliveries={rows:?} retained_out={retained}");
                assert!(lab.cards().is_empty());
                assert_eq!(gateway.world().calls(method).len(), 1);
                observations.push((kind, reply, preempt, correct, retained));
            }
        }
    }
    assert!(
        observations
            .iter()
            .all(|(_, _, _, correct, retained)| *correct && !retained),
        "lost receipts: {observations:?}"
    );
}

#[test]
fn verifier4_cut_before_first_answer_byte_never_confirms_or_retains_any_kind() {
    for kind in ["question", "approval", "batch"] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        let (recording, request, method, settled, decision) = scenario(kind);
        lab.hears(&gateway, recording, request);
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let shared = lab.observer.as_ref().unwrap().local();
        let instance = shared.instance_of(&id).unwrap();
        let (pending, _waiter) = {
            let mut model = shared.model();
            let waiter = model.waiters.remove(&id).unwrap();
            (model.gateway.pending(&id).unwrap().clone(), waiter)
        };
        lab.requests
            .answer(&id, &decision, crate::hooks::now_ms())
            .unwrap();
        let reply = pending.reply(&decision, Scope::Host).unwrap().unwrap();
        let (mut visit, _) = Visit::begin_for(
            &shared,
            &instance,
            &pending.session,
            &id,
            Duration::from_secs(2),
        )
        .unwrap();
        let socket = gateway.world().calls("session.resume").last().unwrap().0;
        let written = answer::verifier3_written(&shared, &pending, &mut visit, |socket| {
            assert!(shared.model.try_lock().is_ok());
            assert!(shared.model().answered.contains_key(&id));
            gateway.play(recording, Some(settled));
            shared.want_poll();
            until("card closes before first byte", || {
                shared.model().gateway.pending(&id).is_none()
            });
            assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
            match &reply {
                Reply::Call(command) => socket.send(command).map(|_| ()),
                Reply::Result(frame) => socket.write(frame),
            }
        });
        assert!(written.is_none());
        assert!(!shared.model().answered.contains_key(&id));
        visit.end();
        until("cut visit ends", || gateway.world().ended.contains(&socket));
        assert!(lab.cards().is_empty());
        assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
        let world = gateway.world();
        if kind == "batch" {
            assert!(!world
                .received
                .iter()
                .any(|(_, f)| f["id"] == "srq-be9ca8d8cb91"));
        } else {
            assert!(world.calls(method).is_empty());
        }
        eprintln!("CUT BEFORE BYTE kind={kind}: no complete frame, no receipt, no retained Out");
    }
}

#[test]
fn verifier4_batch_receipt_waits_for_successful_write_return_after_close() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    let (recording, request, _, _, decision) = scenario("batch");
    lab.hears(&gateway, recording, request);
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().local();
    let instance = shared.instance_of(&id).unwrap();
    let (pending, _waiter) = {
        let mut model = shared.model();
        let waiter = model.waiters.remove(&id).unwrap();
        (model.gateway.pending(&id).unwrap().clone(), waiter)
    };
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    let Reply::Result(frame) = pending.reply(&decision, Scope::Host).unwrap().unwrap() else {
        panic!("not a batch")
    };
    let (mut visit, _) = Visit::begin_for(
        &shared,
        &instance,
        &pending.session,
        &id,
        Duration::from_secs(2),
    )
    .unwrap();
    let written = answer::verifier3_written(&shared, &pending, &mut visit, |socket| {
        socket.write(&frame)?;
        until("batch parsed", || {
            gateway
                .world()
                .received
                .iter()
                .any(|(_, f)| f["id"] == frame["id"])
        });
        shared.want_poll();
        until("poll closes batch before write callback returns", || {
            shared.model().gateway.pending(&id).is_none()
        });
        assert!(
            lab.requests.deliveries(crate::hooks::now_ms()).is_empty(),
            "receipt was emitted before write outcome"
        );
        assert!(!shared.model().answered.get(&id).unwrap().written);
        Ok(())
    });
    assert!(written.is_some());
    let rows = lab.requests.deliveries(crate::hooks::now_ms());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "confirmed");
    assert!(!shared.model().answered.contains_key(&id));
    visit.end();
    eprintln!("BATCH OUTCOME: no receipt before write return, exactly one confirmed afterward");
}

#[test]
fn verifier4_reply_after_poll_close_gives_one_correct_receipt_for_questions_and_approvals() {
    for kind in ["question", "approval"] {
        for reply in [None, Some("already_resolved"), Some("stale_generation")] {
            let mut lab = lab();
            let gateway = Fake::start(&lab.home, "instance-1");
            gateway.world().answers_with = reply;
            lab.follow();
            let (recording, request, method, settled, decision) = scenario(kind);
            lab.hears(&gateway, recording, request);
            let id = lab.card()["id"].as_str().unwrap().to_owned();
            let shared = lab.observer.as_ref().unwrap().local();
            let held = Arc::new(Hold::default());
            gateway.world().hold_next = Some((method, held.clone()));
            lab.requests
                .answer(&id, &decision, crate::hooks::now_ms())
                .unwrap();
            until("gateway holding actual reply", || {
                held.entered.load(Ordering::Acquire)
            });
            if reply == Some("stale_generation") {
                gateway.play(recording, Some(settled));
            }
            shared.want_poll();
            until("poll closes before reply arrives", || {
                shared.model().gateway.pending(&id).is_none()
            });
            assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
            let socket = gateway.world().calls(method)[0].0;
            held.released.store(true, Ordering::Release);
            until("reply handled and visit ended", || {
                gateway.world().ended.contains(&socket)
            });
            let rows = lab.requests.deliveries(crate::hooks::now_ms());
            assert_eq!(rows.len(), 1, "{kind} {reply:?}");
            let expected = if reply.is_none() {
                "confirmed"
            } else {
                "failed"
            };
            assert_eq!(rows[0]["state"], expected, "{kind} {reply:?}");
            assert!(!shared.model().answered.contains_key(&id));
            assert!(lab.cards().is_empty());
            assert_eq!(gateway.world().calls(method).len(), 1);
            eprintln!(
                "EARLY CLOSE kind={kind} reply={reply:?}: one {expected} receipt, no retained Out"
            );
        }
    }
}

#[test]
fn verifier4_partial_batch_write_cut_by_poll_close_never_confirms() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    let (recording, request, _, settled, _) = scenario("batch");
    lab.hears(&gateway, recording, request);
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().local();
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let decision = json!({"kind":"answer","answers":{"q0":{"custom":"x".repeat(8*1024*1024)}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("batch peer stalls", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("batch write is marked", || shared.writing());
    std::thread::sleep(Duration::from_millis(100));
    assert!(shared.writing());
    let socket = gateway.world().calls("session.resume").last().unwrap().0;
    gateway.play(recording, Some(settled));
    shared.want_poll();
    until("poll closes partly written batch", || {
        shared.model().gateway.pending(&id).is_none()
    });
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
    stalled.released.store(true, Ordering::Release);
    until("specific partial batch socket ends", || {
        gateway.world().ended.contains(&socket)
    });
    until("failed batch write forgets its Out", || {
        !shared.model().answered.contains_key(&id)
    });
    assert!(!gateway
        .world()
        .received
        .iter()
        .any(|(_, f)| f["id"] == "srq-be9ca8d8cb91"));
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
    assert!(lab.cards().is_empty());
    eprintln!("PARTIAL BATCH CUT: no complete frame, no receipt, no retained Out");
}
