//! A request allowed automatically showed no card, so what is known to be
//! that request makes no sound (ticket 66): a wait its channel says is for
//! it, by its id, and a transcript sighting of its tool call. Nothing else
//! is silent. A wait that names no request always sounds. What is approved
//! is what core says at each look (its answer confirmed delivered): the
//! decision remembers no approval from the look before (round 6).
use super::*;

const S1: &str = "claude:s1";

/// A session told as waiting with no card.
fn told_waiting(id: &str) -> Value {
    waiting(id, &[])
}

/// One look: `approved` are the requests core holds as approved at this
/// look, and `waits` the requests each session's wait is for, both by
/// `source:session`.
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
    // The same wait at a later look, core still holding the request.
    let named = [(S1, &["r1"][..])];
    let alerts = look_with(&mut tracker, &sessions, &named, &named, T + 700_000);
    assert_eq!(alerts, [], "the same wait sounded late");
    // Another session's wait is its own.
    let both = [told_waiting("s1"), told_waiting("s2")];
    let alerts = look_with(&mut tracker, &both, &named, &named, T + 700_001);
    assert_eq!(alerts, [needs_you("s2")]);
}

/// Round 6. The decision remembers no approval. A wait that was silent
/// while core said its request was approved sounds at the first look
/// where core says so no longer (the answer failed, was settled, or was
/// not heard of in time), though the channel names the same request for
/// it, and whenever that look comes. Once: it is the same wait after.
#[test]
fn a_wait_sounds_once_core_no_longer_says_its_request_is_approved() {
    for later in [2, 600_000, 900_000] {
        let mut tracker = started();
        let sessions = [told_waiting("s1")];
        let named = [(S1, &["r1"][..])];
        assert_eq!(
            look_with(&mut tracker, &sessions, &named, &named, T + 1),
            []
        );
        assert_eq!(
            look_with(&mut tracker, &sessions, &[], &named, T + later),
            [needs_you("s1")],
            "{later} ms on"
        );
        assert_eq!(tracker.take_ended(), [(S1.to_owned(), "r1".to_owned())]);
        assert_eq!(
            look_with(&mut tracker, &sessions, &[], &named, T + later + 1),
            []
        );
    }
}

/// A wait for a request core never said was approved sounds at once,
/// though it names that request alone: an answer on its way is not one.
#[test]
fn a_wait_for_a_request_core_does_not_say_is_approved_sounds() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let named = [(S1, &["r1"][..])];
    assert_eq!(
        look_with(&mut tracker, &sessions, &[], &named, T + 1),
        [needs_you("s1")]
    );
    // Confirmed afterwards: no second sound, and none while it goes on.
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 2),
        []
    );
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 3),
        []
    );
}

/// A card always waits for the person. One that goes by an approved
/// request's name sounds like any card: at the first look, and after the
/// request's own wait passed in silence.
#[test]
fn a_card_that_goes_by_an_approved_requests_name_sounds() {
    let named = [(S1, &["r1"][..])];
    let carded = [waiting("s1", &[("r1", "permission")])];
    let mut tracker = started();
    assert_eq!(
        look_with(&mut tracker, &carded, &named, &named, T + 1),
        [needs_you("s1")]
    );
    let mut tracker = started();
    assert_eq!(
        look_with(&mut tracker, &[told_waiting("s1")], &named, &named, T + 1),
        []
    );
    assert_eq!(
        look_with(&mut tracker, &carded, &[], &named, T + 2),
        [needs_you("s1")]
    );
}

/// A wait with no card that named its requests has sounded. A card that
/// then comes for a request the wait did not name is another request, and
/// sounds; the card of a request it did name is that wait, and does not.
#[test]
fn a_card_the_sounded_wait_did_not_name_is_new_and_one_it_named_is_not() {
    for (card, sounds) in [("r2", 1), ("r1", 0)] {
        let mut tracker = started();
        let named = [(S1, &["r1"][..])];
        assert_eq!(
            look_with(&mut tracker, &[told_waiting("s1")], &[], &named, T + 1),
            [needs_you("s1")]
        );
        let carded = [waiting("s1", &[(card, "question")])];
        let both = [(S1, &["r1", "r2"][..])];
        let alerts = look_with(&mut tracker, &carded, &[], &both, T + 2);
        assert_eq!(alerts.len(), sounds, "{card}");
    }
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

/// A wait that names no request sounds; the approved request's wait takes
/// its place, in silence; then the session waits on something unnamed
/// again, all without being seen at work and with nothing new written.
/// The first wait ended when the second took its place, so the third is
/// new: it sounds, a second time.
#[test]
fn a_wait_that_names_no_request_sounds_again_after_an_approved_ones_took_its_place() {
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
        look_with(&mut tracker, &sessions, &named, &[], T + 3),
        [needs_you("s1")]
    );
    assert_eq!(look_with(&mut tracker, &sessions, &named, &[], T + 4), []);
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
    assert_eq!(tracker.take_ended(), []);
    // The channel names r1 no more; r2's wait goes on.
    look_with(&mut tracker, &sessions, &approved, &[(S1, &["r2"])], T + 2);
    assert_eq!(tracker.take_ended(), [(S1.to_owned(), "r1".to_owned())]);
    assert_eq!(tracker.take_ended(), []);
    // The session is at work again.
    look_with(&mut tracker, &[running("s1")], &approved, &[], T + 3);
    assert_eq!(tracker.take_ended(), [(S1.to_owned(), "r2".to_owned())]);
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
    assert_eq!(look(&mut tracker, "toolu_1", true, T + 60_000), 0);
    assert_eq!(look(&mut tracker, "toolu_2", true, T + 60_001), 1);
    // Round 6: silent only while core says the call is approved. A
    // sighting still there when core holds it no longer (ten minutes
    // after a hook's approval) sounds, once.
    let mut tracker = started();
    assert_eq!(look(&mut tracker, "toolu_1", true, T + 1), 0);
    assert_eq!(look(&mut tracker, "toolu_1", false, T + 600_000), 1);
    assert_eq!(look(&mut tracker, "toolu_1", false, T + 600_001), 0);
    // The control: the same sighting where nothing was approved sounds.
    let mut tracker = started();
    assert_eq!(look(&mut tracker, "toolu_1", false, T + 1), 1);
}

/// A control from round 2, restored. A permission that shows its card at
/// the first look sounds, though another request was allowed automatically
/// in the same session: the approval is silent only for what goes by its
/// own name.
#[test]
fn a_card_seen_at_the_first_look_beside_an_approval_in_its_session_sounds() {
    let mut tracker = started();
    let carded = [waiting("s1", &[("manual-card", "permission")])];
    assert_eq!(
        look_with(&mut tracker, &carded, &[(S1, &["r1"])], &[], T + 1),
        [needs_you("s1")]
    );
    // And where the channel names both for the wait.
    let mut tracker = started();
    let both = [(S1, &["r1", "manual-card"][..])];
    assert_eq!(
        look_with(&mut tracker, &carded, &[(S1, &["r1"])], &both, T + 1),
        [needs_you("s1")]
    );
}

/// A control from round 2, restored. The approved request's wait passes
/// and goes on past ten minutes, core holding the request while its wait
/// is named. A question's card that then comes in it is new, and sounds.
/// (Until round 6 the look at ten minutes was handed no approval and was
/// silent by what the decision remembered. Now core says so itself.)
#[test]
fn a_question_card_late_in_the_approved_requests_wait_sounds() {
    let mut tracker = started();
    let sessions = [told_waiting("s1")];
    let named = [(S1, &["r1"][..])];
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 1),
        []
    );
    assert_eq!(
        look_with(&mut tracker, &sessions, &named, &named, T + 600_001),
        []
    );
    let question = [waiting("s1", &[("late-question", "question")])];
    let both = [(S1, &["r1", "late-question"][..])];
    assert_eq!(
        look_with(&mut tracker, &question, &named, &both, T + 600_002),
        [needs_you("s1")]
    );
}

/// An approved request silences its own session's wait and no other's:
/// the same id named by another session's wait, or by a session of the
/// same id under another source, is not that request.
#[test]
fn an_approved_requests_id_silences_no_other_sessions_or_sources_wait() {
    let mut tracker = started();
    let mut other_source = told_waiting("s1");
    other_source["source"] = json!("codex");
    let rows = [told_waiting("s1"), told_waiting("s2"), other_source];
    let waits = [
        (S1, &["shared-id"][..]),
        ("claude:s2", &["shared-id"][..]),
        ("codex:s1", &["shared-id"][..]),
    ];
    let alerts = look_with(&mut tracker, &rows, &[(S1, &["shared-id"])], &waits, T + 1);
    assert_eq!(alerts, [needs_you("s2"), needs_you("s1")]);
}
