//! A request allowed automatically showed no card, so it makes no sound
//! (ticket 66): the wait its session is told in for a moment, by its
//! hooks or its channel, passes silently while it has no card.
use super::*;

/// A session told as waiting with no request: what its hooks or its
/// channel say while a permission is being asked.
fn told_waiting(id: &str) -> Value {
    waiting(id, &[])
}

fn look_approved(
    tracker: &mut Tracker,
    sessions: &[Value],
    approved: &[&str],
    now: i64,
) -> Vec<(String, String, String)> {
    let before = tracker.items.len();
    let heard = Heard {
        approved: approved
            .iter()
            .map(|key| ((*key).to_owned(), now))
            .collect(),
        ..Heard::default()
    };
    let out = tracker.update(&no_events(), sessions, &heard, &on(), now);
    out["items"].as_array().unwrap()[before..]
        .iter()
        .map(|item| {
            let text = |key: &str| item[key].as_str().unwrap().to_owned();
            (text("kind"), text("sessionId"), text("sound"))
        })
        .collect()
}

/// The control: the same wait, with nothing approved, sounds.
#[test]
fn a_wait_with_no_card_sounds_where_nothing_was_approved() {
    let mut tracker = started();
    let alerts = look_approved(&mut tracker, &[told_waiting("s1")], &[], T + 1);
    assert_eq!(alerts, [needs_you("s1")]);
}

#[test]
fn a_request_approved_automatically_showed_no_card_and_makes_no_sound() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    // Told as waiting just as its permission is allowed with no card.
    let alerts = look_approved(&mut tracker, &sessions, &["claude:s1"], T + 1);
    assert_eq!(alerts, [], "a request that showed no card sounded");
    // Still told as waiting after the approval is no longer noted: the
    // same wait, so still silent.
    let alerts = look_approved(&mut tracker, &sessions, &[], T + 10_000);
    assert_eq!(alerts, [], "the same wait sounded late");
    // Another session's wait is its own.
    let both = [told_waiting("s1"), told_waiting("s2")];
    let alerts = look_approved(&mut tracker, &both, &["claude:s1"], T + 10_001);
    assert_eq!(alerts, [needs_you("s2")]);
}

/// What does show a card still sounds: a question that comes while the
/// approved request's wait is passing, and a permission's card in a
/// session another permission was approved in.
#[test]
fn a_card_in_a_session_just_approved_in_still_sounds() {
    let mut tracker = started();
    let alerts = look_approved(&mut tracker, &[told_waiting("s1")], &["claude:s1"], T + 1);
    assert_eq!(alerts, []);
    let asked = [waiting("s1", &[("card-q", "question")])];
    let alerts = look_approved(&mut tracker, &asked, &["claude:s1"], T + 2);
    assert_eq!(alerts, [needs_you("s1")]);

    let mut tracker = started();
    let carded = [waiting("s2", &[("card-p", "permission")])];
    let alerts = look_approved(&mut tracker, &carded, &["claude:s2"], T + 1);
    assert_eq!(alerts, [needs_you("s2")]);
}

/// The wait that passed goes on past the few seconds the approval is
/// noted for; a question's card that then comes in it is new, and sounds.
#[test]
fn a_card_that_comes_long_after_the_wait_passed_still_sounds() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    assert_eq!(
        look_approved(&mut tracker, &sessions, &["claude:s1"], T + 1),
        []
    );
    assert_eq!(look_approved(&mut tracker, &sessions, &[], T + 10_000), []);
    let asked = [waiting("s1", &[("card-q", "question")])];
    let alerts = look_approved(&mut tracker, &asked, &[], T + 10_001);
    assert_eq!(alerts, [needs_you("s1")]);
    // And a wait told before any approval is still one wait with its card.
    let mut tracker = started();
    assert_eq!(
        look_approved(&mut tracker, &sessions, &[], T + 1),
        [needs_you("s1")]
    );
    assert_eq!(look_approved(&mut tracker, &asked, &[], T + 2), []);
}
