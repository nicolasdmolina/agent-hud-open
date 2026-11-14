//! The third review of ticket 27 (`evidence/27/verifier-report-3.md`):
//! the verifier's three probes (`reviewer3-probes.rs`), tracked as tests.
//! Nothing in them is changed but the formatting; every assertion stands.
//! They count every receipt core was told of, in order
//! (`Requests::reviewer27_notes`). Two failed on `b0720b7`:
//!
//! - `reviewer3_unsent_decision_has_exactly_one_receipt`: one answer the
//!   gateway gives no ticket for was said to have failed twice.
//! - `reviewer3_old_out_must_not_make_an_unwritten_fresh_decision_uncertain`:
//!   a new answer that never got as far as a write was called uncertain,
//!   on the record of the answer before it, whose thread had not yet
//!   returned from its cut write.
//!
//! The third passed there and is a control: a mark that lands after core
//! said a decision is current and before its write is let through.
use super::super::{answer, visit::Visit};
use super::unreachable::{asks, follower, red, started};
use super::{fake::Hold, lab, marked, until};
use agent_hud_agents::hermes::gateway::{Failure, Input, Reply, Scope};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

fn now() -> i64 {
    crate::hooks::now_ms()
}

#[test]
fn reviewer3_unsent_decision_has_exactly_one_receipt() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    gateway.world().grants = false;
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("failed decision noted", || {
        !lab.requests.reviewer27_notes(&id).is_empty()
    });
    let notes = {
        // undelivered holds this lock across all of its receipt calls.
        let model = shared.model();
        assert!(model.gateway.marked(&id));
        lab.requests.reviewer27_notes(&id)
    };
    let wire = gateway.world().calls("clarify.respond").len();
    eprintln!(
        "TRACE one decision, ticket denied: wire_answers={wire} receipts={notes:?} rows={:?}\n{}",
        lab.requests.deliveries(now()),
        lab.log()
    );
    poll.released.store(true, Ordering::Release);
    assert_eq!(wire, 0);
    assert_eq!(
        notes,
        ["failed"],
        "one unsent decision was receipted more than once"
    );
}

#[test]
fn reviewer3_old_out_must_not_make_an_unwritten_fresh_decision_uncertain() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let (pending, instance, old_wait) = {
        let mut model = shared.model();
        let waiter = model.waiters.remove(&id).unwrap();
        lab.requests.answer(&id, &red(), now()).unwrap();
        assert!(waiter.response.recv().unwrap().is_some());
        assert!(lab.requests.current(&id, waiter.wait));
        model.taken.insert(id.clone(), waiter.wait);
        (
            model.gateway.pending(&id).unwrap().clone(),
            model.instance_of(&id).unwrap(),
            waiter.wait,
        )
    };
    let Reply::Call(command) = pending.reply(&red(), Scope::Host).unwrap().unwrap() else {
        panic!("not a call")
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
            let written = answer::verifier27_written_with_wait(
                &shared,
                &pending,
                &mut visit,
                old_wait,
                |socket| {
                    assert!(shared.model.try_lock().is_ok());
                    old.entered.store(true, Ordering::Release);
                    until("old writer released", || {
                        old.released.load(Ordering::Acquire)
                    });
                    socket.send(&command)
                },
            );
            assert!(written.is_none());
            visit.end();
        });
        until("old admitted before first byte", || {
            old.entered.load(Ordering::Acquire)
        });
        let identity = gateway.world().identify.clone().unwrap();
        {
            let mut model = shared.model();
            shared.take(&mut model, Input::Lost(Failure::Closed(1006)));
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
        until("mark clears", || !marked(&lab.card()));
        let fresh = Arc::new(Hold::default());
        *lab.route.held.lock().unwrap() = Some(("prepare", fresh.clone()));
        lab.requests.answer(&id, &red(), now()).unwrap();
        until("fresh held before any visit/write", || {
            fresh.entered.load(Ordering::Acquire)
        });
        lab.route.shut.store(true, Ordering::Release);
        let notes = {
            let mut model = shared.model();
            let fresh_wait = *model.taken.get(&id).unwrap();
            let record_wait = model.answered.get(&id).unwrap().taken;
            assert_ne!(fresh_wait, old_wait);
            assert_eq!(record_wait, old_wait);
            assert!(lab.requests.current(&id, fresh_wait));
            eprintln!("TRACE before second mark: old_wait={old_wait} fresh_wait={fresh_wait} Out.owner={record_wait}; fresh admission has not happened");
            shared.take(&mut model, Input::Lost(Failure::Unreachable));
            lab.requests.reviewer27_notes(&id)
        };
        fresh.released.store(true, Ordering::Release);
        old.released.store(true, Ordering::Release);
        old_writer.join().unwrap();
        eprintln!(
            "TRACE after second mark: wire_answers={} receipts={notes:?} rows={:?}\n{}",
            gateway.world().calls("clarify.respond").len(),
            lab.requests.deliveries(now()),
            lab.log()
        );
        assert_eq!(gateway.world().calls("clarify.respond").len(), 0);
        assert_eq!(
            notes,
            ["uncertain", "failed"],
            "the old answer's Out made the unwritten fresh answer uncertain"
        );
    });
}

#[test]
fn reviewer3_mark_after_core_currency_before_write_admission_never_sends() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let (pending, instance, wait) = {
        let mut model = shared.model();
        let waiter = model.waiters.remove(&id).unwrap();
        lab.requests.answer(&id, &red(), now()).unwrap();
        assert!(waiter.response.recv().unwrap().is_some());
        model.taken.insert(id.clone(), waiter.wait);
        assert!(lab.requests.current(&id, waiter.wait));
        (
            model.gateway.pending(&id).unwrap().clone(),
            model.instance_of(&id).unwrap(),
            waiter.wait,
        )
    };
    let Reply::Call(command) = pending.reply(&red(), Scope::Host).unwrap().unwrap() else {
        panic!("not a call")
    };
    let (mut visit, _) = Visit::begin_for(
        &shared,
        &instance,
        &pending.session,
        &id,
        Duration::from_secs(2),
    )
    .unwrap();
    let identity = gateway.world().identify.clone().unwrap();
    {
        let mut model = shared.model();
        shared.take(&mut model, Input::Lost(Failure::Closed(1006)));
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
    until("reconciled while old visit idle", || !marked(&lab.card()));
    let called = AtomicBool::new(false);
    let result =
        answer::verifier27_written_with_wait(&shared, &pending, &mut visit, wait, |socket| {
            called.store(true, Ordering::Release);
            socket.send(&command)
        });
    visit.end();
    let notes = lab.requests.reviewer27_notes(&id);
    eprintln!("TRACE current=true, then mark/reconnect/reconcile before admission: callback={} write_completed={} answers={} receipts={notes:?}", called.load(Ordering::Acquire), result.is_some(), gateway.world().calls("clarify.respond").len());
    assert!(result.is_none());
    assert!(!called.load(Ordering::Acquire));
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert_eq!(notes, ["failed"]);
    assert_eq!(lab.cards().len(), 1);
}
