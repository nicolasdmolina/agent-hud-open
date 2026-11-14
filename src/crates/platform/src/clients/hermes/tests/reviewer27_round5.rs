//! The fifth review of ticket 27 (`evidence/27/verifier-report-5.md`):
//! the verifier's probe of a card that closes while core hands a
//! decision over (`round5-logs/close-race-probe.rs`), tracked for both
//! ways a card closes, a poll that finds the request settled and a
//! follower that ends with its gateway. Both failed on `9f64650`.
//!
//! Core has taken the decision and waits just before it hands it over
//! (`Requests::reviewer27_hold_acceptance`). The card closes. Core hands
//! the decision over then, while the close still holds the waiter, and
//! `Requests::answer` returns `Ok`. That decision must end with one
//! receipt, failed, and nothing may be sent.
//!
//! What is changed: the probe paused the close through a call from the
//! production code into this file, right after the close had read the
//! waiter's channel and found it empty. The close reads no channel any
//! more: it asks core what it has taken. So it pauses at the place that
//! is left, after core answered and with the waiter still held
//! (`answer::AT_CLOSE`, a place only tests are told of), and the gate is
//! entered whatever was found. The probe's own text stands, with its
//! imports and the one helper it called written out. Every assertion
//! stands.
use super::super::{answer, follower::End};
use super::unreachable::{asks, follower, red, started, ASKED};
use super::{fake::Fake, fake::Hold, lab, until};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

fn now() -> i64 {
    crate::hooks::now_ms()
}

/// Holds a poll at the gateway, so that none ends until it is let go.
fn pause_poll(shared: &super::super::Shared, gateway: &Fake) -> Arc<Hold> {
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    poll
}

/// What `answer::AT_CLOSE` calls: the close of this observer's request.
fn at_close(observer: usize, id: &str) {
    close_read_empty(observer, id, true);
}

// This gate pauses only the disposable copy immediately after Close's try_recv.
static CLOSE_EMPTY: std::sync::Mutex<Option<(usize, String, Arc<Hold>)>> =
    std::sync::Mutex::new(None);
pub(in super::super) fn close_read_empty(observer: usize, id: &str, empty: bool) {
    let gate = CLOSE_EMPTY
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(of, request, _)| *of == observer && request == id && empty)
        .map(|(_, _, gate)| gate.clone());
    if let Some(gate) = gate {
        gate.entered.store(true, Ordering::Release);
        until("close empty read gate released", || {
            gate.released.load(Ordering::Acquire)
        });
    }
}
#[test]
fn reviewer5_successful_send_between_close_empty_read_and_drop_settled_has_one_receipt() {
    successful_send_during_close(false);
}
#[test]
fn reviewer5_successful_send_between_close_empty_read_and_drop_restart_has_one_receipt() {
    successful_send_during_close(true);
}
fn successful_send_during_close(restart: bool) {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    *answer::AT_CLOSE.write().unwrap() = Some(at_close);
    let poll = pause_poll(&shared, &gateway);
    let accepted_gate = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    lab.requests
        .reviewer27_hold_acceptance(Some((id.clone(), accepted_gate.clone())));
    let requests = lab.requests.clone();
    let copy = id.clone();
    let accepted = std::thread::spawn(move || requests.answer(&copy, &red(), now()));
    until("core has taken decision", || {
        accepted_gate.0.load(Ordering::Acquire)
    });
    let close_gate = Arc::new(Hold::default());
    *CLOSE_EMPTY.lock().unwrap() = Some((
        Arc::as_ptr(&shared) as usize,
        id.clone(),
        close_gate.clone(),
    ));
    let ended = if restart {
        let shared = shared.clone();
        Some(std::thread::spawn(move || shared.end(End::Restarted)))
    } else {
        gateway.play(ASKED, Some("clarify.settled"));
        poll.released.store(true, Ordering::Release);
        shared.want_poll();
        None
    };
    until("Close read empty but still owns receiver", || {
        close_gate.entered.load(Ordering::Acquire)
    });
    accepted_gate.1.store(true, Ordering::Release);
    let result = accepted.join().unwrap();
    eprintln!(
        "TRACE acceptance published after Close's empty read: restart={restart} API={result:?}"
    );
    assert!(
        result.is_ok(),
        "this is an actual successfully accepted decision"
    );
    lab.requests.reviewer27_hold_acceptance(None);
    close_gate.released.store(true, Ordering::Release);
    if let Some(ended) = ended {
        ended.join().unwrap();
    }
    until("close released and card absent", || {
        shared.model().gateway.pending(&id).is_none()
    });
    *CLOSE_EMPTY.lock().unwrap() = None;
    std::thread::sleep(Duration::from_millis(150));
    let actual = lab.requests.reviewer27_notes(&id);
    let model = shared.model();
    eprintln!("TRACE successful accepted click lost by close: restart={restart} wire={} receipts={actual:?} waiter={} taken={} Out={} cards={}",gateway.world().calls("clarify.respond").len(),model.waiters.contains_key(&id),model.taken.contains_key(&id),model.answered.contains_key(&id),lab.cards().len());
    assert!(lab.cards().is_empty());
    assert!(gateway.world().calls("clarify.respond").is_empty());
    poll.released.store(true, Ordering::Release);
    assert_eq!(
        actual,
        ["failed"],
        "successful accepted decision dropped between empty read and receiver drop"
    );
}
