//! An automatic approval is silent only once its answer is confirmed
//! delivered (ticket 66, round 6; the round 5 verifier's two probes).
//! Queued, on its way, failed, not confirmed, settled before it left, or
//! not heard of in ten minutes: the request is not approved, and its wait
//! and its card sound as they did before the sync. The sound decision asks
//! core at every look and remembers no approval. Through the whole core,
//! counting the `needs-you` ids as the host plays them, each once.
use super::auto_approved_sound::{Lab, A, THREAD};
use super::*;

fn failed(lab: &Lab, id: &str, receipt: &str) {
    let why = Some("the fixture's reason".to_owned());
    let requests = lab.core.requests();
    requests.note_native_delivery(id, "codex", THREAD, receipt, why, lab.now);
}

/// The round 5 verifier's first probe. A permission is allowed
/// automatically and its wait is seen while the answer is on its way. Half
/// a second later the channel reports that the answer failed. The next
/// look comes just before ten minutes, at ten minutes, and at fifteen: the
/// request still waits for the person, and its wait has sounded, once,
/// whenever that look comes. Before round 6 the look at ten minutes and
/// after found no sound at all.
#[test]
fn a_failed_automatic_answer_has_sounded_whenever_the_next_look_comes() {
    for delay in [599_999, 600_000, 900_000] {
        for receipt in ["failed", "uncertain"] {
            let mut lab = Lab::new(true);
            assert_eq!(lab.look(), ("running".into(), 0, 0));
            assert!(lab.card(A, "permission"));
            lab.told_waiting_for(&[A]);
            let first = lab.look();
            assert_eq!((first.0.as_str(), first.1), ("waiting", 0));
            lab.later(500);
            failed(&lab, A, receipt);
            // No look between the receipt and this one.
            lab.later(delay - 500);
            let at = format!("{receipt}, {delay} ms on");
            assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{at}");
            assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{at}, again");
            assert!(lab.kept().is_empty(), "{at}");
        }
    }
}

/// The same with no look before the receipt: the first look of all comes
/// ten minutes or more after the failure.
#[test]
fn a_failed_automatic_answer_sounds_at_a_first_look_ten_minutes_on() {
    for delay in [599_999, 600_000, 900_000] {
        let mut lab = Lab::new(true);
        assert_eq!(lab.look(), ("running".into(), 0, 0));
        assert!(lab.card(A, "permission"));
        lab.told_waiting_for(&[A]);
        lab.later(500);
        failed(&lab, A, "failed");
        lab.later(delay - 500);
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{delay} ms on");
    }
}

/// While the answer is on its way the request is not approved: a look
/// that sees its wait then sounds, as the card did before the sync. The
/// confirmation that follows makes no second sound, and the wait is
/// silent from then on.
#[test]
fn a_wait_seen_while_the_automatic_answer_is_on_its_way_sounds_once() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    assert!(lab.kept().is_empty(), "approved before it was confirmed");
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.later(200);
    lab.confirmed(A);
    assert_eq!(lab.kept(), [A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.later(200);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// An answer nobody ever reports on is never approved, whenever the wait
/// is first seen: after a second, or after the ten minutes core keeps it.
#[test]
fn an_automatic_answer_nobody_reports_on_never_counts_as_approved() {
    for first_look in [1_000, 599_999, 600_000, 900_000] {
        let mut lab = Lab::new(true);
        assert_eq!(lab.look(), ("running".into(), 0, 0));
        assert!(lab.card(A, "permission"));
        lab.told_waiting_for(&[A]);
        lab.later(first_look);
        assert_eq!(lab.look(), ("waiting".into(), 0, 1), "{first_look} ms");
        // A confirmation after the ten minutes finds nothing to confirm.
        if first_look >= 600_000 {
            lab.confirmed(A);
            assert!(lab.kept().is_empty());
        }
    }
}

/// A confirmed answer whose wait goes on. Core holds the approval as long
/// as the channel says the session's wait is for that request, so the same
/// wait is silent just before ten minutes, at ten, and at fifteen: by what
/// core says at each look, not by anything the decision remembers. When
/// the wait is seen to end, the approval ends with it.
#[test]
fn a_confirmed_answers_wait_stays_silent_past_ten_minutes() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.approved(A);
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
    let mut elapsed = 0;
    for at in [599_999, 600_000, 900_000] {
        lab.later(at - elapsed);
        elapsed = at;
        assert_eq!(lab.look(), ("waiting".into(), 0, 0), "{at} ms on");
        assert_eq!(lab.kept(), [A], "{at} ms on");
    }
    lab.not_waiting();
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.kept().is_empty());
    // The same id named by a wait again is approved no more: it sounds.
    lab.later(100);
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// A confirmed answer no wait names is kept ten minutes and no longer.
#[test]
fn a_confirmed_answer_no_wait_names_is_kept_ten_minutes() {
    let mut lab = Lab::new(true);
    lab.look();
    lab.approved(A);
    lab.later(599_999);
    assert_eq!(lab.kept(), [A]);
    lab.later(1);
    assert!(lab.kept().is_empty());
    // A wait that names it afterwards is not approved: it sounds.
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
}

/// The round 5 verifier's second probe, at core. A permission is allowed
/// automatically, and its channel settles it before the answer left (its
/// listener stopped). Approve automatically is turned off and the same
/// request comes back: it shows its card, and the card sounds, once.
/// Before round 6 the card showed with no sound.
#[test]
fn a_request_settled_before_its_answer_left_sounds_when_its_card_shows() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.core.requests().settle_native(A);
    lab.core
        .apply_setting("autoApprove", &json!(false))
        .unwrap();
    assert!(!lab.card(A, "permission"), "allowed with the rule off");
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// The same where the answer was confirmed and its wait passed in silence
/// before the settlement: the card that then shows waits for the person,
/// so it sounds. Nothing a look decided about the approval is remembered.
#[test]
fn a_confirmed_request_shown_again_as_a_card_sounds() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    lab.approved(A);
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 0));
    lab.core.requests().settle_native(A);
    assert!(lab.kept().is_empty(), "kept after it was settled");
    lab.core
        .apply_setting("autoApprove", &json!(false))
        .unwrap();
    assert!(!lab.card(A, "permission"));
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// And where the unconfirmed request's wait had already sounded: the card
/// of that same request is the same wait, one sound in all, as before the
/// sync (its card sounded once).
#[test]
fn the_card_of_a_wait_that_already_sounded_is_the_same_wait() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    lab.core.requests().settle_native(A);
    lab.core
        .apply_setting("autoApprove", &json!(false))
        .unwrap();
    assert!(!lab.card(A, "permission"));
    assert_eq!(lab.look(), ("waiting".into(), 1, 1));
}

/// A request whose answer is not confirmed has no card, and its wait
/// sounds. A question then asked is another request, as its channel says
/// by naming both: its card sounds too, as it did before the sync beside
/// the first one's card.
#[test]
fn a_question_card_beside_an_unconfirmed_requests_wait_sounds() {
    let mut lab = Lab::new(true);
    assert_eq!(lab.look(), ("running".into(), 0, 0));
    assert!(lab.card(A, "permission"));
    lab.told_waiting_for(&[A]);
    assert_eq!(lab.look(), ("waiting".into(), 0, 1));
    let question = "codex-desktop-question:2";
    assert!(!lab.card(question, "question"));
    lab.told_waiting_for(&[A, question]);
    assert_eq!(lab.look(), ("waiting".into(), 1, 2));
    assert_eq!(lab.look(), ("waiting".into(), 1, 2));
}
