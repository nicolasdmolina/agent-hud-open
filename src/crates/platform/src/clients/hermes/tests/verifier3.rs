//! The independent verifier's probes from the third review of ticket 13
//! (`evidence/13/verifier-3-extra.patch`), kept as written but for the
//! first. It asked that a whole answer never be read by the gateway after
//! a stop or a card closing. The coordinator ruled the boundary: an answer
//! is never written after the cutoff, and a frame that was fully queued
//! before it was sent in time and may still be read. The probe now asserts
//! exactly that. One bound in the last probe is wider: the half-second
//! write limit must fail the answer within 5 s, not 1.5 s. Run alone it
//! takes 501 to 506 ms; in the whole suite on a machine at load 44 the
//! test's threads were scheduled late and it took 1.7 to 3.3 s. Pretend
//! gateways and temporary homes only.
use super::super::{answer, visit::Visit, Pace};
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use agent_hud_agents::hermes::gateway::{Input, Reply, Scope};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

#[test]
fn verifier3_a_frame_fully_queued_before_the_cutoff_may_still_be_read() {
    let mut observations = Vec::new();
    for change in ["stop", "close"] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let shared = lab.observer.as_ref().unwrap().shared.clone();
        let stalled = Arc::new(Hold::default());
        gateway.world().stall_after_resume = Some(stalled.clone());
        lab.requests
            .answer(&id, &decision(), crate::hooks::now_ms())
            .unwrap();
        until("small-answer peer stops reading", || {
            stalled.entered.load(Ordering::Acquire)
        });
        until("whole small frame was enqueued before cutoff", || {
            lab.log().contains("answer sent id=")
        });
        assert!(!shared.writing());
        let socket = gateway.world().calls("session.resume").last().unwrap().0;
        assert!(gateway.world().calls("clarify.respond").is_empty());
        match change {
            "stop" => shared.stop(),
            "close" => {
                gateway.play("question-answered-elsewhere", Some("clarify.settled"));
                until("small answer card closes", || {
                    shared.model().gateway.pending(&id).is_none()
                });
            }
            _ => unreachable!(),
        }
        assert!(gateway.world().calls("clarify.respond").is_empty());
        stalled.released.store(true, Ordering::Release);
        until("peer parses buffered complete frame", || {
            !gateway.world().calls("clarify.respond").is_empty()
        });
        until("specific buffered-answer visit ends", || {
            gateway.world().ended.contains(&socket)
        });
        let frames = gateway.world().calls("clarify.respond");
        eprintln!(
            "WHOLE FRAME {change}: parsed_after_cutoff={} frame={:?}",
            frames.len(),
            frames[0].1
        );
        observations.push((change, frames.len()));
    }
    // Written once, before the cutoff, and read once after it: nothing was
    // written again, and nothing was written late.
    assert_eq!(observations, [("stop", 1), ("close", 1)]);
}

fn decision() -> Value {
    json!({"kind":"answer", "answers":{"e1bc9d4fa6":{"selected":["red"]}}})
}

#[test]
fn verifier3_idle_visits_cannot_write_or_resume_after_stop_close_or_instance_change() {
    for change in ["stop", "close", "instance"] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let shared = lab.observer.as_ref().unwrap().shared.clone();
        let instance = shared.instance_of(&id).unwrap();
        let pending = shared.model().gateway.pending(&id).unwrap().clone();
        let (mut visit, _) = Visit::begin_for(
            &shared,
            &instance,
            &pending.session,
            &id,
            Duration::from_secs(2),
        )
        .unwrap();
        let resumed = gateway.world().calls("session.resume");
        let socket = resumed.last().unwrap().0;
        let before = resumed.iter().filter(|(n, _)| *n == socket).count();
        assert!(!shared.writing());
        match change {
            "stop" => shared.stop(),
            "close" => {
                gateway.play("question-answered-elsewhere", Some("clarify.settled"));
                until("poll closes idle visit's card", || {
                    shared.model().gateway.pending(&id).is_none()
                });
            }
            "instance" => {
                let mut identity = gateway.world().identify.clone().unwrap();
                identity["result"]["instance_id"] = json!("instance-2");
                shared.apply(Input::Identified {
                    answer: &identity,
                    home: lab.home.to_str().unwrap(),
                });
            }
            _ => unreachable!(),
        }
        assert!(!shared.current(&id, &instance), "{change}");
        let written: Option<()> =
            visit.write(|_| panic!("stale idle visit entered writer after {change}"));
        assert!(written.is_none());
        visit.resume_again(Duration::from_millis(100));
        visit.end();
        until("idle visit ended", || {
            gateway.world().ended.contains(&socket)
        });
        let world = gateway.world();
        assert_eq!(
            world
                .calls("session.resume")
                .iter()
                .filter(|(n, _)| *n == socket)
                .count(),
            before,
            "{change}"
        );
        assert!(world.calls("clarify.respond").is_empty(), "{change}");
        eprintln!("IDLE {change}: no answer or retry resume, peer ended");
    }
}

#[test]
fn verifier3_stop_between_mark_and_first_byte_cuts_the_registered_socket() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    let instance = shared.instance_of(&id).unwrap();
    let pending = shared.model().gateway.pending(&id).unwrap().clone();
    let (mut visit, _) = Visit::begin_for(
        &shared,
        &instance,
        &pending.session,
        &id,
        Duration::from_secs(2),
    )
    .unwrap();
    let socket_id = gateway.world().calls("session.resume").last().unwrap().0;
    let Reply::Call(command) = pending.reply(&decision(), Scope::Host).unwrap().unwrap() else {
        panic!("not a call")
    };
    let sent = visit.write(|socket| {
        assert!(
            shared.model.try_lock().is_ok(),
            "model lock held when write callback starts"
        );
        shared.stop();
        socket.send(&command)
    });
    assert!(sent.is_none());
    visit.end();
    until("marked socket ended", || {
        gateway.world().ended.contains(&socket_id)
    });
    assert!(gateway.world().calls("clarify.respond").is_empty());
    eprintln!("MARK/STOP: no complete answer, peer ended");
}

#[test]
fn verifier3_instance_change_cuts_a_backpressured_answer() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    let instance = shared.instance_of(&id).unwrap();
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let decision =
        json!({"kind":"answer", "answers":{"e1bc9d4fa6":{"custom":"x".repeat(8*1024*1024)}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("peer stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("answer is marked writing", || shared.writing());
    std::thread::sleep(Duration::from_millis(100));
    let socket = gateway.world().calls("session.resume").last().unwrap().0;
    let mut identity = gateway.world().identify.clone().unwrap();
    identity["result"]["instance_id"] = json!("instance-2");
    let began = Instant::now();
    shared.apply(Input::Identified {
        answer: &identity,
        home: lab.home.to_str().unwrap(),
    });
    assert!(began.elapsed() < Duration::from_millis(250));
    assert!(!shared.current(&id, &instance));
    stalled.released.store(true, Ordering::Release);
    until("specific answer socket ended", || {
        gateway.world().ended.contains(&socket)
    });
    assert!(gateway.world().calls("clarify.respond").is_empty());
    eprintln!("INSTANCE: no complete answer, peer ended");
}

#[test]
fn verifier3_successful_batch_keeps_receipt_if_poll_closes_before_sent_bookkeeping() {
    for preempt in [false, true] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        lab.hears(&gateway, "question-batch-answered-here", "clarify");
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let shared = lab.observer.as_ref().unwrap().shared.clone();
        let instance = shared.instance_of(&id).unwrap();
        let (pending, _waiter) = {
            let mut model = shared.model();
            let waiter = model.waiters.remove(&id).unwrap();
            (model.gateway.pending(&id).unwrap().clone(), waiter)
        };
        let decision = json!({"kind":"answer", "answers":{"q0":{"selected":["green"]}}});
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
            // Hold this writer at a legal preemption boundary: send returned,
            // but Going::written has not acquired the model for Input::Sent.
            if preempt {
                until("gateway parsed the complete batch answer", || {
                    gateway
                        .world()
                        .received
                        .iter()
                        .any(|(_, f)| f["id"] == frame["id"])
                });
                shared.want_poll();
                until("poll closes answered batch", || {
                    shared.model().gateway.pending(&id).is_none()
                });
            }
            Ok(())
        });
        assert!(written.is_some());
        visit.end();
        until("batch card closes", || {
            shared.model().gateway.pending(&id).is_none()
        });
        assert!(lab.cards().is_empty());
        let deliveries = lab.requests.deliveries(crate::hooks::now_ms());
        eprintln!("BATCH preempt={preempt} deliveries={deliveries:?}");
        assert!(
            deliveries
                .iter()
                .any(|d| d["id"] == id && d["state"] == "confirmed"),
            "complete accepted batch lost its confirmation receipt"
        );
    }
}

#[test]
fn verifier3_default_write_budget_stops_a_stalled_frame_without_later_flush() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.observer = Some(
        super::super::Observer::start_at(
            lab.home.clone(),
            &lab.data,
            lab.requests.clone(),
            Arc::new(|| {}),
            Pace {
                write: Pace::default().write,
                ..super::quick()
            },
        )
        .unwrap(),
    );
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let decision =
        json!({"kind":"answer", "answers":{"e1bc9d4fa6":{"custom":"x".repeat(8*1024*1024)}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("default-budget peer stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("default-budget answer starts writing", || shared.writing());
    let socket = gateway.world().calls("session.resume").last().unwrap().0;
    let began = Instant::now();
    assert_eq!(lab.delivery(&id)["state"], "failed");
    let took = began.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "default-budget failure took {took:?}"
    );
    stalled.released.store(true, Ordering::Release);
    until("default-budget answer socket ended", || {
        gateway.world().ended.contains(&socket)
    });
    assert!(gateway.world().calls("clarify.respond").is_empty());
    eprintln!(
        "DEFAULT BUDGET: failed in {} ms, no complete answer, peer ended",
        took.as_millis()
    );
}
