//! The sixth review of ticket 27 (`evidence/27/verifier-report-6.md`):
//! the verifier's three probes (`round6-logs/platform-probes.rs`),
//! tracked as tests. Nothing in them is changed but the formatting;
//! every assertion stands. One failed on `7243d26`:
//!
//! - `reviewer6_real_off_between_ending_flag_and_model_cleanup_has_no_receipt`.
//!   Client hooks turn off for real. The end has said it is ending and
//!   waits, at a lock it takes on its way, before it has the model. An
//!   answer that was held back finds its way shut just then. It used to
//!   mark the card, give it back, and note that the answer failed. Off
//!   leaves no receipt.
//!
//! The other two passed there and are controls: a follower that is not
//! ending still takes a real loss, and still finds its gateway gone.
use super::unreachable::{asks, follower, red, started};
use super::{fake::Hold, lab, marked, until};
use agent_hud_agents::hermes::gateway::{Failure, Input};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

#[test]
fn reviewer6_active_follower_still_processes_real_loss() {
    for failure in [Failure::Unreachable, Failure::Closed(1006)] {
        let mut lab = lab();
        let gateway = started(&mut lab);
        let id = asks(&lab, &gateway);
        let shared = follower(&lab);
        let held = Arc::new(Hold::default());
        *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
        lab.requests
            .answer(&id, &red(), crate::hooks::now_ms())
            .unwrap();
        until("answer held", || held.entered.load(Ordering::Acquire));
        lab.route.shut.store(true, Ordering::Release);
        assert!(shared.active());
        shared.apply(Input::Lost(failure.clone()));
        assert!(shared.model().gateway.marked(&id));
        assert!(marked(&lab.card()));
        assert_eq!(lab.requests.reviewer27_notes(&id), ["failed"]);
        held.released.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(lab.requests.reviewer27_notes(&id), ["failed"]);
        assert!(gateway.world().calls("clarify.respond").is_empty());
        eprintln!("TRACE active follower loss={failure:?}: marked, receipts=[failed], wire=0");
    }
}

#[test]
fn reviewer6_active_follower_still_detects_a_gone_gateway() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    gateway.stop();
    until("real stopped gateway is gone and its card marked", || {
        shared.gone.load(Ordering::Acquire) && shared.model().gateway.marked(&id)
    });
    assert!(shared.active());
    assert!(marked(&lab.card()));
    assert!(lab.requests.reviewer27_notes(&id).is_empty());
    assert!(lab
        .requests
        .answer(&id, &red(), crate::hooks::now_ms())
        .is_err());
    eprintln!("TRACE real socket/control stop: active=true gone=true marked=true; click refused; receipts=[]");
}

#[test]
fn reviewer6_real_off_between_ending_flag_and_model_cleanup_has_no_receipt() {
    use super::super::follower::End;
    use std::time::Instant;
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let shared = follower(&lab);
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.requests
        .answer(&id, &red(), crate::hooks::now_ms())
        .unwrap();
    until("answer held before socket open", || {
        held.entered.load(Ordering::Acquire)
    });
    assert!(shared.model().taken.contains_key(&id));
    // Real End::Off sets active=false then takes stopping. Hold that mutex
    // to pause it at an ordinary lock boundary, before model cleanup.
    let stopping = shared.stopping.0.lock().unwrap();
    let ending = shared.clone();
    let end = std::thread::spawn(move || ending.end(End::Off));
    until("real end set active=false", || !shared.active());
    assert!(!shared.model().stopped);
    held.released.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(1);
    while lab.requests.reviewer27_notes(&id).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = lab.requests.reviewer27_notes(&id);
    drop(stopping);
    end.join().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let after = lab.requests.reviewer27_notes(&id);
    eprintln!("TRACE real End::Off, held at stopping mutex: before_cleanup={before:?} after_cleanup={after:?} wire={} cards={}\n{}", gateway.world().calls("clarify.respond").len(), lab.cards().len(), lab.log());
    assert!(lab.cards().is_empty());
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert!(
        after.is_empty(),
        "Off must produce no receipt for its outstanding accepted decision"
    );
}
