//! A wait with no card that is seen to end and then seen again (ticket 66,
//! round 4). A channel's waiting flag has no time of its own, so such a
//! wait is known by the time of the session's last event, and two waits
//! with nothing written between them would go by one name. The name is
//! forgotten when the wait is seen to end, so the second one sounds. A
//! wait known by a time of its own (a hook's attention) keeps its name.
use super::*;

fn alerts(tracker: &mut Tracker, sessions: &[Value], now: i64) -> usize {
    look(tracker, &no_events(), sessions, &on(), now).len()
}

/// A session whose hooks reported, at `at`, that it needs the person.
fn attention(id: &str, at: &str) -> Value {
    let mut session = waiting(id, &[]);
    session["liveFacts"]["attention"] = json!({"at": at});
    session
}

#[test]
fn a_wait_seen_to_end_and_begin_again_sounds_again_though_nothing_was_written() {
    let mut tracker = started();
    let waits = [waiting("s1", &[])];
    assert_eq!(alerts(&mut tracker, &waits, T + 1), 1);
    // It only goes on: one sound.
    assert_eq!(alerts(&mut tracker, &waits, T + 2), 0);
    // Seen at work, then waiting again, the row's times as they were.
    assert_eq!(alerts(&mut tracker, &[running("s1")], T + 3), 0);
    assert_eq!(alerts(&mut tracker, &waits, T + 4), 1);
    assert_eq!(alerts(&mut tracker, &waits, T + 5), 0);
    // Gone from the list altogether, and back.
    assert_eq!(alerts(&mut tracker, &[], T + 6), 0);
    assert_eq!(alerts(&mut tracker, &waits, T + 7), 1);
}

/// The control: a wait its hooks reported is known by the time of that
/// report. The same report after the session was seen at work is the same
/// wait told again, as before; a later report is a new one.
#[test]
fn a_wait_known_by_its_own_time_is_one_wait_however_often_it_is_seen() {
    let mut tracker = started();
    let first = [attention("s1", "2027-01-15T08:00:05Z")];
    assert_eq!(alerts(&mut tracker, &first, T + 1), 1);
    assert_eq!(alerts(&mut tracker, &[running("s1")], T + 2), 0);
    assert_eq!(alerts(&mut tracker, &first, T + 3), 0);
    assert_eq!(alerts(&mut tracker, &[running("s1")], T + 4), 0);
    let second = [attention("s1", "2027-01-15T08:00:09Z")];
    assert_eq!(alerts(&mut tracker, &second, T + 5), 1);
}

/// A wait that got its card is still that wait, and its borrowed name
/// goes when it ends: the next wait with no card sounds.
#[test]
fn the_name_a_wait_borrowed_goes_when_it_ends_with_its_card() {
    let mut tracker = started();
    assert_eq!(alerts(&mut tracker, &[waiting("s1", &[])], T + 1), 1);
    let carded = [waiting("s1", &[("card-1", "question")])];
    assert_eq!(alerts(&mut tracker, &carded, T + 2), 0);
    assert_eq!(alerts(&mut tracker, &[running("s1")], T + 3), 0);
    assert_eq!(alerts(&mut tracker, &[waiting("s1", &[])], T + 4), 1);
}
