//! The first review of ticket 27 (`evidence/27/verifier-report.md`): the
//! verifier's three probes (`reviewer-probes.rs`), which all failed on
//! `ea5a937`, tracked as tests. Nothing in them is changed but the
//! formatting; every assertion stands.
//!
//! - The two loss probes go into the answer's own write path, and lose
//!   the connection where its check has passed and no byte is written
//!   yet: the mark has to cut that write.
//! - The held probe keeps an answer's subscription back, loses the
//!   connection, lets a poll find the request waiting again, and lets the
//!   answer go on: it must not be sent, and the card must be there to
//!   answer anew.
use super::super::{answer, visit::Visit};
use super::unreachable::{asks, follower, red, started};
use super::{fake::Hold, lab, until};
use agent_hud_agents::hermes::gateway::{Failure, Input, Reply, Scope};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

#[test]
fn reviewer27_loss_between_write_check_and_first_byte_cuts_the_answer() {
    write_after_loss(Failure::Closed(1006));
}

#[test]
fn reviewer27_unclear_connection_between_check_and_write_cuts_the_answer() {
    write_after_loss(Failure::Unreachable);
}

fn write_after_loss(failure: Failure) {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
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
    let Reply::Call(command) = pending.reply(&red(), Scope::Host).unwrap().unwrap() else {
        panic!("not a call")
    };
    // Make the connection remain lost after the cutoff.
    lab.route.shut.store(true, Ordering::Release);
    let sent = answer::verifier3_written(&shared, &pending, &mut visit, |socket| {
        // This callback is the real write-outside-lock boundary. No answer
        // bytes have been written by this callback yet.
        assert!(shared.model.try_lock().is_ok());
        shared.apply(Input::Lost(failure));
        assert!(shared.model().gateway.marked(&id));
        eprintln!(
            "TRACE loss processed, card marked, writing={}, answer bytes=0",
            shared.writing()
        );
        socket.send(&command)
    });
    visit.end();
    until("answer socket ends", || {
        gateway.world().ended.contains(&socket_id)
    });
    let count = gateway.world().calls("clarify.respond").len();
    eprintln!(
        "TRACE after mark: write_completed={}, received_answers={count}",
        sent.is_some()
    );
    assert!(
        sent.is_none(),
        "an answer was fully written after the unreachable mark"
    );
    assert_eq!(count, 0);
}

#[test]
fn reviewer27_a_held_unsent_answer_is_not_sent_automatically_after_reconcile() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    // Keep the poll from taking the next subscription hold. That hold
    // must belong to the actual answer thread, not a snapshot visit.
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("the poll is held", || poll.entered.load(Ordering::Acquire));
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.resume", held.clone()));
    lab.requests
        .answer(&id, &red(), crate::hooks::now_ms())
        .unwrap();
    until("answer subscription is held", || {
        held.entered.load(Ordering::Acquire)
    });
    let answer_socket = gateway.world().calls("session.resume").last().unwrap().0;
    assert!(gateway.world().calls("clarify.respond").is_empty());
    let identity = gateway.world().identify.clone().unwrap();
    {
        let mut model = shared.model();
        shared.take(&mut model, Input::Lost(Failure::Closed(1006)));
        assert!(model.gateway.marked(&id));
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
    until("first accepted poll clears the mark", || {
        !shared.model().gateway.marked(&id)
    });
    assert!(gateway.world().calls("clarify.respond").is_empty());
    eprintln!("TRACE loss marked the request; first accepted poll unmarked it; answer still held; core_cards={}", lab.cards().len());
    // There is no second click. The held first answer resumes here.
    held.released.store(true, Ordering::Release);
    until("held answer socket ends", || {
        gateway.world().ended.contains(&answer_socket)
    });
    let count = gateway.world().calls("clarify.respond").len();
    eprintln!(
        "TRACE no second decision: received_answers={count}, cards={}, log=\n{}",
        lab.cards().len(),
        lab.log()
    );
    assert_eq!(
        count, 0,
        "the held answer was automatically sent after reconnect reconciliation"
    );
    assert_eq!(
        lab.cards().len(),
        1,
        "the original request must be offered again"
    );
}
