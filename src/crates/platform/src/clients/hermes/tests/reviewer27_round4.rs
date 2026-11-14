//! The fourth review of ticket 27 (`evidence/27/verifier-report-4.md`):
//! the verifier's probes (`round4-logs/reviewer4-probes.rs`), tracked as
//! tests. Each counts every receipt core was told of for one decision
//! (`Requests::reviewer27_notes`): one, whichever way the decision ends.
//! Four failed on `15793d4`, each with no receipt at all:
//!
//! - a request that settles, or a follower that ends with its gateway,
//!   after core took the decision and before anything of the answer was
//!   written;
//! - a poll that closes the card after the answer was written, and a
//!   reply that then refuses it, or never comes.
//!
//! One probe is changed, as the coordinator ruled: turning client hooks
//! off stays the one end that leaves no receipt, as ticket 13 has it. Its
//! probe asked for one. It is kept under the name
//! `reviewer4_off_before_write_has_no_receipt` and asserts none, and for
//! that the three endings share one function that is told what to
//! expect. Nothing else is changed but the formatting; the other
//! assertions stand. The rest passed on `15793d4` and are controls.
use super::super::follower::End;
use super::unreachable::{asks, follower, red, started, ASKED};
use super::{fake::Hold, lab, until};
use agent_hud_agents::hermes::gateway::{Failure, Input};
use serde_json::json;
use std::sync::{atomic::Ordering, Arc};

fn now() -> i64 {
    crate::hooks::now_ms()
}

#[test]
fn reviewer4_settled_before_write_has_one_receipt() {
    closed_before_write("settled", &["failed"]);
}

#[test]
fn reviewer4_restarted_before_write_has_one_receipt() {
    closed_before_write("restarted", &["failed"]);
}

#[test]
fn reviewer4_off_before_write_has_no_receipt() {
    closed_before_write("off", &[]);
}

fn closed_before_write(ending: &str, expected: &[&str]) {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let held = Arc::new(Hold::default());
    gateway.world().hold_too = Some(("session.resume", held.clone()));
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("real answer subscription held", || {
        held.entered.load(Ordering::Acquire)
    });
    let socket = gateway.world().calls("session.resume").last().unwrap().0;
    {
        let model = shared.model();
        assert!(model.taken.contains_key(&id));
        assert!(!model.answered.contains_key(&id));
    }
    match ending {
        "settled" => {
            gateway.play(ASKED, Some("clarify.settled"));
            poll.released.store(true, Ordering::Release);
            shared.want_poll();
            until("poll settled the card", || {
                lab.log().contains(&format!("close id={id} why=Settled"))
            });
        }
        "restarted" => shared.end(End::Restarted),
        "off" => shared.end(End::Off),
        _ => unreachable!(),
    }
    poll.released.store(true, Ordering::Release);
    held.released.store(true, Ordering::Release);
    until("real answer socket ended", || {
        gateway.world().ended.contains(&socket)
    });
    let notes = lab.requests.reviewer27_notes(&id);
    let wire = gateway.world().calls("clarify.respond").len();
    let model = shared.model();
    eprintln!("TRACE accepted click ending={ending}: wire={wire} receipts={notes:?} taken={} Out={} cards={}\n{}", model.taken.contains_key(&id), model.answered.contains_key(&id), lab.cards().len(), lab.log());
    assert_eq!(wire, 0);
    assert!(!model.taken.contains_key(&id));
    assert!(!model.answered.contains_key(&id));
    assert!(lab.cards().is_empty());
    assert_eq!(
        notes, expected,
        "one accepted user decision ended with no receipt"
    );
}

#[test]
fn reviewer4_sent_refused_and_unconfirmed_each_have_one_receipt() {
    for (reply, expected) in [
        (None, "confirmed"),
        (Some("invalid_answer"), "failed"),
        (Some("silence"), "uncertain"),
    ] {
        let mut lab = lab();
        let gateway = started(&mut lab);
        let id = asks(&lab, &gateway);
        let shared = follower(&lab);
        let poll = Arc::new(Hold::default());
        gateway.world().hold_next = Some(("session.active_list", poll.clone()));
        shared.want_poll();
        until("poll held", || poll.entered.load(Ordering::Acquire));
        gateway.world().answers_with = reply;
        lab.requests.answer(&id, &red(), now()).unwrap();
        until("receipt recorded", || {
            !lab.requests.reviewer27_notes(&id).is_empty()
        });
        until("answer visit ended", || shared.model().visits.is_empty());
        let notes = lab.requests.reviewer27_notes(&id);
        let wire = gateway.world().calls("clarify.respond").len();
        poll.released.store(true, Ordering::Release);
        eprintln!("TRACE reply={reply:?}: wire={wire} receipts={notes:?}");
        assert_eq!(wire, 1);
        assert_eq!(notes, [expected]);
    }
}

#[test]
fn reviewer4_failed_write_has_one_receipt() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let decision =
        json!({"kind":"answer","answers":{"e1bc9d4fa6":{"custom":"x".repeat(8*1024*1024)}}});
    lab.requests.answer(&id, &decision, now()).unwrap();
    until("gateway stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("failed write receipted", || {
        !lab.requests.reviewer27_notes(&id).is_empty()
    });
    until("answer writer finished", || {
        shared.model().visits.is_empty()
    });
    let notes = lab.requests.reviewer27_notes(&id);
    stalled.released.store(true, Ordering::Release);
    poll.released.store(true, Ordering::Release);
    eprintln!(
        "TRACE failed full write: wire={} receipts={notes:?}",
        gateway.world().calls("clarify.respond").len()
    );
    assert_eq!(notes, ["failed"]);
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

#[test]
fn reviewer4_mark_during_write_has_one_receipt() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let decision =
        json!({"kind":"answer","answers":{"e1bc9d4fa6":{"custom":"x".repeat(8*1024*1024)}}});
    lab.requests.answer(&id, &decision, now()).unwrap();
    until("gateway stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("real answer write admitted", || {
        shared.writing() && shared.model().answered.contains_key(&id)
    });
    lab.route.shut.store(true, Ordering::Release);
    shared.take(&mut shared.model(), Input::Lost(Failure::Closed(1006)));
    until("cut writer finished", || shared.model().visits.is_empty());
    let notes = lab.requests.reviewer27_notes(&id);
    stalled.released.store(true, Ordering::Release);
    poll.released.store(true, Ordering::Release);
    eprintln!(
        "TRACE mark during real answer write: wire={} receipts={notes:?}",
        gateway.world().calls("clarify.respond").len()
    );
    assert_eq!(notes, ["uncertain"]);
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

#[test]
fn reviewer4_settled_before_refused_reply_has_one_receipt() {
    closed_before_reply("invalid_answer", "failed");
}

#[test]
fn reviewer4_settled_before_unconfirmed_reply_has_one_receipt() {
    closed_before_reply("silence", "uncertain");
}

fn closed_before_reply(reply: &'static str, expected: &str) {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let poll = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", poll.clone()));
    shared.want_poll();
    until("poll held", || poll.entered.load(Ordering::Acquire));
    let held = Arc::new(Hold::default());
    gateway.world().hold_too = Some(("clarify.respond", held.clone()));
    gateway.world().answers_with = Some(reply);
    lab.requests.answer(&id, &red(), now()).unwrap();
    until("real answer reply held", || {
        held.entered.load(Ordering::Acquire)
    });
    let socket = gateway.world().calls("clarify.respond")[0].0;
    gateway.play(ASKED, Some("clarify.settled"));
    poll.released.store(true, Ordering::Release);
    shared.want_poll();
    until("poll settled the card before reply", || {
        lab.log().contains(&format!("close id={id} why=Settled"))
    });
    assert!(lab.requests.reviewer27_notes(&id).is_empty());
    held.released.store(true, Ordering::Release);
    until("answer socket ended after reply", || {
        gateway.world().ended.contains(&socket)
    });
    let notes = lab.requests.reviewer27_notes(&id);
    let model = shared.model();
    eprintln!(
        "TRACE poll close before reply={reply}: wire={} receipts={notes:?} taken={} Out={}",
        gateway.world().calls("clarify.respond").len(),
        model.taken.contains_key(&id),
        model.answered.contains_key(&id)
    );
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
    assert!(!model.answered.contains_key(&id));
    assert_eq!(
        notes,
        [expected],
        "written answer lost its receipt after poll closure"
    );
}
