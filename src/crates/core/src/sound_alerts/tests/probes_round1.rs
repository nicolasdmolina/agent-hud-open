//! The probes of ticket 75's round 1 verifier (`evidence/75/verify/v75.rs`),
//! asserted. Counts are what the host plays (`lab.rs`).
use super::lab::*;
use super::*;

#[test]
fn a_later_wait_a_day_after_the_first_ended_sounds_once() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.run(100);
        lab.idle(DAY + 1);
        let later = (lab.wait(1, kind), lab.wait(1_000, kind));
        assert_eq!((first, later), (1, (1, 0)), "{kind:?}");
    }
}

#[test]
fn a_sleep_over_a_day_that_ends_a_wait_or_does_not() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.wait(1, kind);
        let gone = lab.run(DAY + 5);
        let later = lab.wait(1, kind);
        assert_eq!((first, gone, later), (1, 0, 1), "{kind:?}");
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        let after = (lab.wait(3 * DAY, kind), lab.wait(1_000, kind));
        assert_eq!((first, after), (1, (0, 0)), "{kind:?}");
    }
}

#[test]
fn a_wait_that_goes_on_sounds_once_and_so_does_a_second() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let mut pings: usize = (0..80).map(|_| lab.wait(HOUR, kind)).sum();
        pings += (0..3).map(|_| lab.wait(DAY + 7, kind)).sum::<usize>();
        assert_eq!(pings, 1, "{kind:?}");
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.run(100);
        let second = lab.wait(100, kind);
        let more: usize = (0..80).map(|_| lab.wait(HOUR, kind)).sum();
        assert_eq!((first, second, more), (1, 1, 0), "{kind:?}");
    }
}

#[test]
fn the_third_wait_after_a_long_second_sounds() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let (first, _, second) = (lab.wait(1, kind), lab.run(100), lab.wait(100, kind));
        let extra: usize = (0..30).map(|_| lab.wait(HOUR, kind)).sum();
        lab.run(100);
        let third = lab.wait(100, kind);
        assert_eq!((first, second, extra, third), (1, 1, 0, 1), "{kind:?}");
    }
}

#[test]
fn the_third_wait_after_the_first_alert_rolled_off_sounds() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.run(100);
        lab.idle(DAY + 1);
        let second = lab.wait(1, kind);
        let rows = || Some(row(kind, None));
        let extra: usize = (0..40)
            .map(|_| lab.see(60_000, rows(), kind, 1, true))
            .sum();
        lab.wait(100, kind);
        lab.run(100);
        let third = lab.wait(100, kind);
        assert_eq!((first, second, extra, third), (1, 1, 0, 1), "{kind:?}");
    }
}

#[test]
fn the_third_wait_after_sound_off_and_on_sounds() {
    let kind = Kind::Bare;
    let mut lab = Lab::new();
    let first = lab.wait(1, kind);
    lab.run(100);
    lab.idle(DAY + 1);
    let second = lab.wait(1, kind);
    lab.see(100, Some(row(kind, None)), kind, 0, false);
    let back = lab.wait(100, kind) + lab.wait(100, kind);
    lab.run(100);
    let third = lab.wait(100, kind);
    assert_eq!((first, second, back, third), (1, 1, 0, 1));
}

#[test]
fn ten_later_waits_a_day_apart_each_sound() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let mut pings = Vec::new();
        for _ in 0..10 {
            pings.push(lab.wait(1, kind));
            lab.run(100);
            lab.idle(DAY + 1);
        }
        assert_eq!(pings, [1; 10], "{kind:?}");
    }
}

#[test]
fn what_is_held_for_300_sessions_over_60_days_stays_small() {
    let mut tracker = started();
    let (mut most, mut now) = ((0, 0), T);
    for round in 0..600 {
        let id = format!("s{}", round % 300);
        now += 1_000;
        tracker.update(
            &no_events(),
            &[waiting(&id, &[])],
            &Heard::default(),
            &on(),
            now,
        );
        now += 1_000;
        tracker.update(&no_events(), &[running(&id)], &Heard::default(), &on(), now);
        now += DAY / 10;
        most = (
            most.0.max(tracker.ends.len()),
            most.1.max(tracker.decided.len()),
        );
    }
    println!("most counts held: {}; most names held: {}", most.0, most.1);
    assert!(most.0 <= 12 && most.1 <= 24, "{most:?}");
    tracker.update(&no_events(), &[], &Heard::default(), &off(), now + DAY);
    assert_eq!((tracker.ends.len(), tracker.decided.len()), (0, 0));
}

#[test]
fn another_session_a_wait_after_a_write_and_a_late_card_sound() {
    let mut lab = Lab::new();
    let first = lab.wait(1, Kind::Bare);
    lab.run(100);
    lab.idle(DAY + 1);
    let other = lab.see(1, Some(waiting("s2", &[])), Kind::Bare, 0, true);
    let written = row(Kind::Bare, Some("2027-01-16T09:00:00Z"));
    let later = lab.see(100, Some(written), Kind::Bare, 0, true);
    assert_eq!((first, other, later), (1, 1, 1));
    let mut lab = Lab::new();
    let named = lab.wait(1, Kind::Named);
    lab.run(100);
    lab.idle(DAY + 1);
    assert_eq!((named, lab.wait(1, Kind::Card)), (1, 1));
}

#[test]
fn a_relaunch_in_a_second_wait_and_between_two_waits() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("in.json");
    let mut lab = Lab::kept(&file, T);
    lab.idle(0);
    let (first, _, second) = (
        lab.wait(1, Kind::Bare),
        lab.run(100),
        lab.wait(100, Kind::Bare),
    );
    lab.wait(100, Kind::Bare);
    let mut lab = Lab::kept(&file, lab.now + 60_000);
    let again = lab.wait(0, Kind::Bare);
    let extra: usize = (0..30).map(|_| lab.wait(HOUR, Kind::Bare)).sum();
    lab.run(100);
    let third = lab.wait(100, Kind::Bare);
    assert_eq!((first, second, again, extra, third), (1, 1, 0, 0, 1));
    for gap in [HOUR, DAY + 1] {
        let file = folder.path().join(format!("between-{gap}.json"));
        let mut lab = Lab::kept(&file, T);
        lab.idle(0);
        let first = lab.wait(1, Kind::Bare);
        lab.run(100);
        let mut lab = Lab::kept(&file, lab.now + gap);
        lab.idle(0);
        assert_eq!((first, lab.wait(1, Kind::Bare)), (1, 1), "{gap}");
    }
}
