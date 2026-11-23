//! A request allowed automatically showed no card, so what is known to be
//! that request makes no sound (ticket 66): a wait its channel says is for
//! it, by its id, and a transcript sighting of its tool call. Nothing else
//! is silent. A wait that names no request always sounds.
use super::*;

const S1: &str = "claude:s1";

/// A session told as waiting with no card.
fn told_waiting(id: &str) -> Value {
    waiting(id, &[])
}

/// One look: `approved` are the requests allowed with no card, and `waits`
/// the requests each session's wait is for, both by `source:session`.
fn look_with(
    tracker: &mut Tracker,
    sessions: &[Value],
    approved: &[(&str, &[&str])],
    waits: &[(&str, &[&str])],
    now: i64,
) -> Vec<(String, String, String)> {
    let before = tracker.items.len();
    let by_session = |list: &[(&str, &[&str])]| {
        list.iter()
            .map(|(key, names)| {
                let names = names.iter().map(|name| (*name).to_owned()).collect();
                ((*key).to_owned(), names)
            })
            .collect()
    };
    let heard = Heard {
        approved: by_session(approved),
        waits: by_session(waits),
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

/// The control: a wait that names no request, with nothing approved.
#[test]
fn a_wait_with_no_card_sounds_where_nothing_was_approved() {
    let mut tracker = started();
    let alerts = look_with(&mut tracker, &[told_waiting("s1")], &[], &[], T + 1);
    assert_eq!(alerts, [needs_you("s1")]);
}

#[test]
fn a_wait_for_the_approved_request_makes_no_sound() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let alerts = look_with(
        &mut tracker,
        &sessions,
        &[(S1, &["r1"])],
        &[(S1, &["r1"])],
        T + 1,
    );
    assert_eq!(alerts, [], "a request that showed no card sounded");
    // The same wait at a later look, where core keeps the request no
    // longer (its bound ran out): still that wait, still silent.
    let alerts = look_with(&mut tracker, &sessions, &[], &[(S1, &["r1"])], T + 700_000);
    assert_eq!(alerts, [], "the same wait sounded late");
    // Another session's wait is its own.
    let both = [told_waiting("s1"), told_waiting("s2")];
    let alerts = look_with(&mut tracker, &both, &[], &[(S1, &["r1"])], T + 700_001);
    assert_eq!(alerts, [needs_you("s2")]);
}

/// A wait that names no request always sounds, though a request was just
/// approved in its session, and whichever came first.
#[test]
fn a_wait_that_names_no_request_sounds_though_one_was_approved() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let alerts = look_with(&mut tracker, &sessions, &[(S1, &["r1"])], &[], T + 1);
    assert_eq!(alerts, [needs_you("s1")]);
    // Once: the same wait at the next look, the request still kept.
    let alerts = look_with(&mut tracker, &sessions, &[(S1, &["r1"])], &[], T + 2);
    assert_eq!(alerts, []);
}

/// A wait for a request that was not approved, or for an approved one and
/// another beside it, is not the approved request's alone: it sounds.
#[test]
fn a_wait_for_more_than_approved_requests_sounds() {
    for named in [&["r2"][..], &["r1", "r2"][..]] {
        let mut tracker = started();
        let alerts = look_with(
            &mut tracker,
            &[told_waiting("s1")],
            &[(S1, &["r1"])],
            &[(S1, named)],
            T + 1,
        );
        assert_eq!(alerts, [needs_you("s1")], "{named:?}");
    }
}

/// The approved request's wait passes; the session goes on waiting, now
/// for something its channel does not name. That is a new wait: it sounds.
#[test]
fn a_wait_that_names_no_request_after_the_approved_one_is_new_and_sounds() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let named = [(S1, &["r1"][..])];
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 1),
        []
    );
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &[], T + 2),
        [needs_you("s1")]
    );
    assert_eq!(look_with(&mut tracker, &sessions, &named, &[], T + 3), []);
}

/// A wait already told, then found to be an approved request's: it has
/// sounded once, and sounds no second time while it goes on.
#[test]
fn a_wait_told_before_its_request_was_approved_sounds_once() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let named = [(S1, &["r1"][..])];
    assert_eq!(
        look_with(&mut tracker, &sessions, &[], &[], T + 1),
        [needs_you("s1")]
    );
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 2),
        []
    );
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 3),
        []
    );
}

/// What does show a card still sounds: a question that comes while the
/// approved request's wait passes.
#[test]
fn a_card_in_a_wait_that_passed_still_sounds() {
    let mut tracker = started();
    let named = [(S1, &["r1"][..])];
    let alerts = look_with(&mut tracker, &[told_waiting("s1")], &named, &named, T + 1);
    assert_eq!(alerts, []);
    let asked = [waiting("s1", &[("card-q", "question")])];
    let both = [(S1, &["r1", "card-q"][..])];
    assert_eq!(
        look_with(&mut tracker, &asked, &named, &both, T + 10_000),
        [needs_you("s1")]
    );
    // And a wait told before any approval is still one wait with its card.
    let mut tracker = started();
    assert_eq!(
        look_with(&mut tracker, &[told_waiting("s1")], &[], &[], T + 1),
        [needs_you("s1")]
    );
    assert_eq!(look_with(&mut tracker, &asked, &[], &[], T + 2), []);
}

/// The tracker says which approved requests' waits it saw end, by their
/// ids, once: a wait that names the request no more, and a session that
/// waits no more. A request whose wait it never saw is not among them.
#[test]
fn the_look_says_which_approved_requests_waits_ended() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let approved = [(S1, &["r1", "r2", "never-seen"][..])];
    look_with(
        &mut tracker,
        &sessions,
        &approved,
        &[(S1, &["r1", "r2"])],
        T + 1,
    );
    assert_eq!(tracker.take_ended(), Vec::<String>::new());
    // The channel names r1 no more; r2's wait goes on.
    look_with(&mut tracker, &sessions, &approved, &[(S1, &["r2"])], T + 2);
    assert_eq!(tracker.take_ended(), ["r1"]);
    assert_eq!(tracker.take_ended(), Vec::<String>::new());
    // The session is at work again.
    look_with(&mut tracker, &[running("s1")], &approved, &[], T + 3);
    assert_eq!(tracker.take_ended(), ["r2"]);
}

/// What goes by an approved request's own names never sounds: here a
/// transcript sighting of the tool call a hook's callback allowed. A
/// sighting of another call in the same session is another request.
#[test]
fn a_sighting_of_the_approved_call_makes_no_sound_and_another_calls_does() {
    let look = |tracker: &mut Tracker, call: &str, approved: bool, now: i64| {
        let rid = format!("log:claude:s1:{call}");
        let sessions = [waiting("s1", &[(rid.as_str(), "permission")])];
        let kept: &[(&str, &[&str])] = if approved {
            &[(S1, &["call:toolu_1"])]
        } else {
            &[]
        };
        look_with(tracker, &sessions, kept, &[], now).len()
    };
    let mut tracker = started();
    assert_eq!(look(&mut tracker, "toolu_1", true, T + 1), 0);
    assert_eq!(look(&mut tracker, "toolu_1", false, T + 60_000), 0);
    assert_eq!(look(&mut tracker, "toolu_2", false, T + 60_001), 1);
    // The control: the same sighting where nothing was approved sounds.
    let mut tracker = started();
    assert_eq!(look(&mut tracker, "toolu_1", false, T + 1), 1);
}
