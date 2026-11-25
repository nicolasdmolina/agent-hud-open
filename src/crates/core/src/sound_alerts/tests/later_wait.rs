//! A wait seen to end, and a later one more than a day after, with
//! nothing written between them (Agent HUD Fork, ticket 75). The later
//! wait goes by a name the earlier one's alert, still among the items the
//! snapshot carries, does not: the host plays an alert id once while its
//! last snapshot carries it (`host/src/system/sound.rs`, `fresh`), so the
//! same id again would be silent. A wait that only goes on sounds once.
use super::*;

const HOUR: i64 = 3_600_000;

/// The `needs-you` ids the snapshot carries now.
fn carried(tracker: &Tracker) -> HashSet<String> {
    let items = tracker.items.iter().filter(|i| i["kind"] == "needs-you");
    items
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect()
}

/// One look; `waits` and `asks` are what the channel says waits in `s1`.
fn told(tracker: &mut Tracker, sessions: &[Value], waits: &[&str], asks: bool, now: i64) {
    let key = "claude:s1".to_owned();
    let named: Vec<String> = waits.iter().map(|w| (*w).to_owned()).collect();
    let asked = named.iter().map(|w| (w.clone(), w.clone())).collect();
    let heard = Heard {
        waits: HashMap::from([(key.clone(), named)]),
        asks: if asks {
            HashMap::from([(key, asked)])
        } else {
            HashMap::new()
        },
        ..Heard::default()
    };
    tracker.update(&no_events(), sessions, &heard, &on(), now);
}

/// A wait sounds and is seen to end; a day later, with nothing written, a
/// new wait begins. It must sound under an id the first one did not use.
fn a_day_later(waits: &[&str], asks: bool) {
    let mut tracker = started();
    let wait = [waiting("s1", &[])];
    told(&mut tracker, &wait, waits, asks, T + 1);
    let first = carried(&tracker);
    assert_eq!(first.len(), 1);
    told(&mut tracker, &[running("s1")], &[], asks, T + 101);
    told(&mut tracker, &[], &[], asks, T + 101 + DAY + 1);
    told(&mut tracker, &wait, waits, asks, T + 101 + DAY + 2);
    let later = carried(&tracker);
    assert_eq!(
        later.difference(&first).count(),
        1,
        "{first:?} then {later:?}"
    );
}

#[test]
fn a_wait_with_no_card_a_day_after_the_last_one_ended_sounds_under_a_new_id() {
    a_day_later(&[], false);
}

#[test]
fn a_named_request_with_no_card_a_day_after_it_ended_sounds_under_a_new_id() {
    a_day_later(&["req-1"], false);
}

#[test]
fn an_ask_with_no_card_a_day_after_it_ended_sounds_under_a_new_id() {
    a_day_later(&["req-1"], true);
}

/// No look between the last that saw the wait and the first, over a day
/// later, that sees it end (a Mac asleep): the end is still marked.
#[test]
fn a_wait_seen_to_end_only_after_a_day_without_a_look_lets_the_next_sound() {
    let mut tracker = started();
    let wait = [waiting("s1", &[])];
    told(&mut tracker, &wait, &[], false, T + 1);
    told(&mut tracker, &wait, &[], false, T + 2);
    let first = carried(&tracker);
    told(&mut tracker, &[running("s1")], &[], false, T + 2 + DAY + 1);
    told(&mut tracker, &wait, &[], false, T + 2 + DAY + 2);
    let later = carried(&tracker);
    assert_eq!(
        later.difference(&first).count(),
        1,
        "{first:?} then {later:?}"
    );
}

/// The control: a wait that only goes on sounds once across a day, looked
/// at every hour, and across one gap of more than a day.
#[test]
fn a_wait_that_only_goes_on_across_a_day_sounds_once() {
    let mut tracker = started();
    let wait = [waiting("s1", &[])];
    for hour in 1..=30 {
        told(&mut tracker, &wait, &[], false, T + hour * HOUR);
    }
    assert_eq!(tracker.items.len(), 1);
    told(&mut tracker, &wait, &[], false, T + 31 * HOUR + DAY + 1);
    told(&mut tracker, &wait, &[], false, T + 32 * HOUR + DAY + 1);
    assert_eq!(tracker.items.len(), 1);
}

/// A relaunch more than a day after the earlier run's wait: the later run
/// starts from the file, and its own waits still never share an id.
#[test]
fn after_a_relaunch_a_wait_a_day_after_the_last_one_ended_sounds_under_a_new_id() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    for start in [T, T + 2 * DAY] {
        // Relaunched: the file is read, and the snapshot carries nothing.
        let mut tracker = Tracker::kept(file.clone(), start);
        told(&mut tracker, &[], &[], false, start);
        let wait = [waiting("s1", &[])];
        told(&mut tracker, &wait, &[], false, start + 1);
        let first = carried(&tracker);
        assert_eq!(first.len(), 1);
        told(&mut tracker, &[running("s1")], &[], false, start + 101);
        told(&mut tracker, &[], &[], false, start + 101 + DAY + 1);
        told(&mut tracker, &wait, &[], false, start + 101 + DAY + 2);
        let later = carried(&tracker);
        assert_eq!(
            later.difference(&first).count(),
            1,
            "{first:?} then {later:?}"
        );
        told(
            &mut tracker,
            &[running("s1")],
            &[],
            false,
            start + 101 + DAY + 3,
        );
    }
}

/// What outlives a wait seen to end is a count of the waits under its
/// name, which goes a day later with the name.
#[test]
fn a_count_of_ended_waits_outlives_the_wait_and_goes_after_a_day() {
    let mut tracker = started();
    told(&mut tracker, &[waiting("s1", &[])], &[], false, T + 1);
    told(&mut tracker, &[running("s1")], &[], false, T + 2);
    let counts: Vec<u32> = tracker.ends.values().map(|(count, _)| *count).collect();
    assert_eq!((counts, tracker.decided.len()), (vec![1], 1));
    told(&mut tracker, &[], &[], false, T + 2 + DAY);
    assert_eq!((tracker.ends.len(), tracker.decided.len()), (0, 0));
}
