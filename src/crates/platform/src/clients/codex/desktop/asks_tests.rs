//! Two requests that wait one after the other in one session, with no
//! state between that says it is at work, each sound, whichever of them
//! has a card (Agent HUD Fork, ticket 74; ticket 66's round 4 verifier
//! probe). The same request seen again, its card withdrawn and restored,
//! still sounds once.
use super::*;
use std::collections::BTreeSet;

/// One look: the session's state, the cards shown, the `needs-you` ids.
fn ids(replay: &mut Replay) -> (String, usize, BTreeSet<String>) {
    let snapshot = replay.core.request("request_snapshot", json!({})).unwrap();
    let rows = snapshot["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
    let cards = snapshot["approvals"].as_array().unwrap().len();
    let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = alerts.iter().filter(|item| item["kind"] == "needs-you");
    let ids = needs.filter_map(|item| item["id"].as_str().map(str::to_owned));
    let state = row["state"].as_str().unwrap().to_owned();
    (state, cards, ids.collect())
}

/// A Replay with Approve automatically off: every request waits for the
/// person.
fn by_hand() -> Replay {
    let mut replay = Replay::new();
    let off = json!({"key": "autoApprove", "value": false});
    replay.core.request("set_setting", off).unwrap();
    replay
}

/// Request #7 waits; in the next state #7 is done and #8 waits. One of the
/// two is of a kind the channel shows a card for, the other not.
fn one_after_the_other(first_shown: bool) {
    let mut replay = by_hand();
    let mut first = state(&["waitingOnApproval"], true);
    let mut next = first.clone();
    next["requests"][0]["id"] = json!(8);
    let unshown = if first_shown { &mut next } else { &mut first };
    unshown["requests"][0]["params"]["_meta"]["connector_id"] = json!("fixture-unsupported");
    replay.streams(first.clone());
    let before = ids(&mut replay);
    assert_eq!(
        (&before.0[..], before.1, before.2.len()),
        ("waiting", usize::from(first_shown), 1)
    );
    let mut done = first["requests"][0].clone();
    done["completed"] = json!(true);
    next["requests"].as_array_mut().unwrap().push(done);
    replay.streams(next);
    let after = ids(&mut replay);
    assert_eq!(
        (&after.0[..], after.1),
        ("waiting", usize::from(!first_shown))
    );
    assert_eq!(
        after.2.difference(&before.2).count(),
        1,
        "first shown: {first_shown}; #8 made no sound of its own"
    );
    assert_eq!(ids(&mut replay), after, "the same look twice");
    replay.observer.stop();
}

#[test]
fn a_card_after_a_request_with_none_that_is_done_sounds() {
    one_after_the_other(false);
}

#[test]
fn a_request_with_no_card_after_a_card_that_is_done_sounds() {
    one_after_the_other(true);
}

/// The control: the same card, its list unreadable for one state and then
/// restored, sounds once.
#[test]
fn the_same_card_withdrawn_and_restored_sounds_once() {
    let mut replay = by_hand();
    let present = state(&["waitingOnApproval"], true);
    replay.streams(present.clone());
    let first = ids(&mut replay);
    assert_eq!((&first.0[..], first.1, first.2.len()), ("waiting", 1, 1));
    let mut unreadable = present.clone();
    unreadable["requests"] = Value::Null;
    replay.streams(unreadable);
    let withdrawn = ids(&mut replay);
    assert_eq!((&withdrawn.0[..], withdrawn.1), ("waiting", 0));
    assert_eq!(withdrawn.2, first.2);
    replay.streams(present);
    assert_eq!(ids(&mut replay), first);
    replay.observer.stop();
}

/// The control: a request the channel shows no card for, the same one in
/// every state, sounds once while it waits; seen to end (a look finds the
/// session at work) and to wait again, it sounds again, as it did before
/// ticket 74.
#[test]
fn a_request_with_no_card_sounds_once_and_again_after_its_wait_is_seen_to_end() {
    let mut replay = by_hand();
    let mut present = state(&["waitingOnApproval"], true);
    present["requests"][0]["params"]["_meta"]["connector_id"] = json!("fixture-unsupported");
    replay.streams(present.clone());
    let first = ids(&mut replay);
    assert_eq!((&first.0[..], first.1, first.2.len()), ("waiting", 0, 1));
    replay.streams(present.clone());
    assert_eq!(ids(&mut replay), first, "the same state again");
    replay.streams(state(&[], false));
    assert_eq!(ids(&mut replay).0, "running");
    replay.streams(present);
    let again = ids(&mut replay);
    assert_eq!((&again.0[..], again.1, again.2.len()), ("waiting", 0, 2));
}
