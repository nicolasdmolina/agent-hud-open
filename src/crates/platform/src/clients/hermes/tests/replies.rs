//! Hermes's reply to an answer is looked at and applied in one hold of the
//! model's lock. The handler looks whether a poll already closed the card
//! and, if not, gives the reply to the model; a poll that got in between
//! would close the card with nobody left to give its receipt.
//!
//! `verifier4.rs` checks the orderings on either side of that. This checks
//! the place itself: at the moment the reply is about to be applied to an
//! open card (`answer::AT_APPLY`), another thread tries the model's lock
//! and must find it held. It only tries, so it never waits on the handler
//! and the handler never waits on a poll: a handler that holds the lock
//! correctly cannot hang this test. A handler that let go of the model
//! after looking (two holds) is seen with the lock free, and one with the
//! check taken out is seen not to have been asked at all.
use super::super::{answer, Shared};
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

/// What the look from another thread saw.
#[derive(Default)]
struct Seen {
    looked: AtomicBool,
    held: AtomicBool,
}

/// The answers being watched, by observer and request: tests that run side
/// by side share request ids.
type Watched = (usize, String, Arc<Shared>, Arc<Seen>);
static WATCH: Mutex<Vec<Watched>> = Mutex::new(Vec::new());

/// Called on the answer's thread where the reply is about to be applied.
fn look(observer: usize, id: &str) {
    let watched = WATCH
        .lock()
        .unwrap()
        .iter()
        .find(|(of, request, _, _)| *of == observer && request == id)
        .map(|(_, _, shared, seen)| (shared.clone(), seen.clone()));
    let Some((shared, seen)) = watched else {
        return;
    };
    // Held at every one of several tries: the answers' watch takes the
    // lock for a moment now and then, which one try alone could meet.
    let looker = std::thread::spawn(move || {
        let held = (0..5).all(|_| {
            let held = shared.model.try_lock().is_err();
            std::thread::sleep(Duration::from_millis(1));
            held
        });
        seen.held.store(held, Ordering::Release);
        seen.looked.store(true, Ordering::Release);
    });
    let _ = looker.join();
}

#[test]
fn a_reply_is_applied_with_the_model_still_held_from_the_look_at_its_card() {
    let kinds = [
        (
            "question-answered-here",
            "clarify.request",
            "clarify.settled",
            json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}}),
        ),
        (
            "approval-answered-once",
            "approval.request",
            "approval.settled",
            json!({"kind": "allow"}),
        ),
    ];
    for (recording, request, settled, decision) in kinds {
        for reply in [None, Some("already_resolved"), Some("stale_generation")] {
            let mut lab = lab();
            let gateway = Fake::start(&lab.home, "instance-1");
            gateway.world().answers_with = reply;
            lab.follow();
            lab.hears(&gateway, recording, request);
            let id = lab.card()["id"].as_str().unwrap().to_owned();
            let shared = lab.observer.as_ref().unwrap().local();
            // No poll closes the card first: the reply meets it open, which
            // is the branch that gives the reply to the model.
            let poll = Arc::new(Hold::default());
            gateway.world().hold_next = Some(("session.active_list", poll.clone()));
            shared.want_poll();
            until("the poll is held", || poll.entered.load(Ordering::Acquire));
            // A request refused as stale is over in the gateway too.
            if reply == Some("stale_generation") {
                gateway.play(recording, Some(settled));
            }
            let seen = Arc::new(Seen::default());
            let observer = Arc::as_ptr(&shared) as usize;
            *answer::AT_APPLY.write().unwrap() = Some(look);
            let watched = (observer, id.clone(), shared.clone(), seen.clone());
            WATCH.lock().unwrap().push(watched);
            lab.requests
                .answer(&id, &decision, crate::hooks::now_ms())
                .unwrap();
            let expected = if reply.is_none() {
                "confirmed"
            } else {
                "failed"
            };
            let receipt = lab.delivery(&id);
            WATCH
                .lock()
                .unwrap()
                .retain(|(of, _, _, _)| *of != observer);
            poll.released.store(true, Ordering::Release);
            assert_eq!(receipt["state"], expected, "{recording} {reply:?}");
            assert!(
                seen.looked.load(Ordering::Acquire),
                "{recording} {reply:?}: the reply was applied somewhere the test is not told of"
            );
            assert!(
                seen.held.load(Ordering::Acquire),
                "{recording} {reply:?}: the model was let go between the look at the card and the reply"
            );
            // The poll let go at the end finds nothing waiting.
            std::thread::sleep(Duration::from_millis(100));
            assert!(lab.cards().is_empty(), "{recording} {reply:?}");
            assert_eq!(lab.requests.deliveries(crate::hooks::now_ms()).len(), 1);
        }
    }
}
