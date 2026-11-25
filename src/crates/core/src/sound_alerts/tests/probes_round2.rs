//! The probes of ticket 75's round 2 verifier (`evidence/75/verify2/w75.rs`),
//! asserted. Counts are what the host plays (`lab.rs`).
use super::lab::*;
use super::*;

/// A later wait, a relaunch a minute into it, and its end `after` later.
#[test]
fn the_wait_after_a_relaunch_early_in_a_later_wait_sounds() {
    let folder = tempfile::tempdir().unwrap();
    for kind in NO_CARD {
        for after in [60_000, HOUR, 23 * HOUR, 25 * HOUR] {
            let file = folder.path().join(format!("{kind:?}-{after}.json"));
            let mut lab = Lab::kept(&file, T);
            lab.idle(0);
            let first = lab.wait(1, kind);
            lab.run(100);
            lab.idle(DAY + 1);
            let second = lab.wait(1, kind);
            lab.wait(30_000, kind);
            let mut lab = Lab::kept(&file, lab.now + 30_000);
            let again = lab.wait(0, kind) + lab.wait(1_000, kind) + lab.wait(after, kind);
            lab.run(100);
            let third = lab.wait(100, kind);
            lab.run(100);
            let fourth = lab.wait(100, kind);
            let got = (first, second, again, third, fourth);
            assert_eq!(got, (1, 1, 0, 1, 1), "{kind:?} {after}");
        }
    }
}

/// A second wait `before` hours long, a relaunch, its end `after` later.
#[test]
fn the_wait_after_a_relaunch_late_in_a_second_wait_sounds() {
    let folder = tempfile::tempdir().unwrap();
    for kind in NO_CARD {
        for (before, after) in [(1, 1), (25, 1), (25, 23), (25, 25), (49, 1)] {
            let file = folder
                .path()
                .join(format!("{kind:?}-{before}-{after}.json"));
            let mut lab = Lab::kept(&file, T);
            lab.idle(0);
            let (first, _, second) = (lab.wait(1, kind), lab.run(100), lab.wait(100, kind));
            let mut extra: usize = (0..before).map(|_| lab.wait(HOUR, kind)).sum();
            let mut lab = Lab::kept(&file, lab.now + 60_000);
            extra += lab.wait(0, kind);
            extra += (0..after).map(|_| lab.wait(HOUR, kind)).sum::<usize>();
            lab.run(100);
            let third = lab.wait(100, kind);
            let got = (first, second, extra, third);
            assert_eq!(got, (1, 1, 0, 1), "{kind:?} {before}h {after}h");
        }
    }
}

#[test]
fn sleeps_around_a_second_wait() {
    for kind in NO_CARD {
        for others in [0, 40] {
            let mut lab = Lab::new();
            let (first, _, second) = (lab.wait(1, kind), lab.run(100), lab.wait(100, kind));
            lab.see(100, Some(row(kind, None)), kind, others, true);
            let woke = lab.run(DAY + 5);
            let third = lab.wait(1, kind);
            lab.run(100);
            let fourth = lab.wait(100, kind);
            let got = (first, second, woke, third, fourth);
            assert_eq!(got, (1, 1, 0, 1, 1), "{kind:?} {others}");
            let mut lab = Lab::new();
            let first = lab.wait(1, kind);
            lab.run(100);
            let second = lab.wait(3 * DAY, kind);
            lab.see(100, Some(row(kind, None)), kind, others, true);
            let still = lab.wait(3 * DAY, kind);
            lab.run(100);
            let third = lab.wait(100, kind);
            assert_eq!(
                (first, second, still, third),
                (1, 1, 0, 1),
                "{kind:?} {others}"
            );
        }
    }
}

#[test]
fn sound_off_while_a_later_wait_starts_or_between_two() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.run(100);
        lab.idle(DAY + 1);
        lab.see(1, Some(row(kind, None)), kind, 0, false);
        let back = lab.wait(100, kind);
        lab.run(100);
        let third = lab.wait(100, kind);
        lab.run(100);
        lab.idle(DAY + 1);
        let fourth = lab.wait(100, kind);
        assert_eq!((first, back, third, fourth), (1, 0, 1, 1), "{kind:?}");
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.run(100);
        lab.see(100, None, kind, 0, false);
        lab.idle(DAY + 1);
        let second = lab.wait(1, kind);
        lab.run(100);
        let third = lab.wait(100, kind);
        assert_eq!((first, second, third), (1, 1, 1), "{kind:?}");
    }
}

#[test]
fn a_hook_time_that_comes_and_goes_is_one_wait() {
    let mut lab = Lab::new();
    let first = lab.wait(1, Kind::Bare);
    let mut own = row(Kind::Bare, None);
    own["liveFacts"]["attention"] = json!({"at":"2027-01-15T08:00:05Z"});
    let with = lab.see(100, Some(own), Kind::Bare, 0, true);
    let without = lab.wait(100, Kind::Bare);
    lab.run(100);
    let second = lab.wait(100, Kind::Bare);
    assert_eq!((first, with, without, second), (1, 0, 0, 1));
}

#[test]
fn cards_that_go_on_and_cards_after_waits_of_their_request() {
    let mut lab = Lab::new();
    let mut pings: usize = (0..72).map(|_| lab.wait(HOUR, Kind::Card)).sum();
    pings += lab.wait(2 * DAY, Kind::Card);
    assert_eq!(pings, 1);
    for others in [0, 40] {
        let mut lab = Lab::new();
        let named = lab.wait(1, Kind::Named);
        lab.run(100);
        lab.idle(DAY + 1);
        let card = lab.wait(1, Kind::Card);
        let mut extra = lab.see(100, Some(row(Kind::Card, None)), Kind::Card, others, true);
        extra += (0..30).map(|_| lab.wait(HOUR, Kind::Card)).sum::<usize>();
        lab.run(100);
        let soon = lab.wait(100, Kind::Named);
        lab.run(100);
        lab.idle(DAY + 1);
        let late = lab.wait(1, Kind::Card);
        assert_eq!(
            (named, card, extra, soon, late),
            (1, 1, 0, 1, 1),
            "{others}"
        );
    }
    let mut lab = Lab::new();
    let (first, _, second) = (
        lab.wait(1, Kind::Bare),
        lab.run(100),
        lab.wait(100, Kind::Bare),
    );
    let carded = || Some(waiting("s1", &[("card-9", "permission")]));
    let mut extra: usize = (0..30)
        .map(|_| lab.see(HOUR, carded(), Kind::Bare, 0, true))
        .sum();
    extra += lab.wait(100, Kind::Bare);
    lab.run(100);
    let third = lab.wait(100, Kind::Bare);
    assert_eq!((first, second, extra, third), (1, 1, 0, 1));
}

#[test]
fn memory_and_sound_over_many_waits_and_flaps() {
    for kind in NO_CARD {
        let mut lab = Lab::new();
        let mut silent = 0;
        let mut most = (0, 0);
        for n in 0..400 {
            silent += usize::from(lab.wait(if n % 5 == 0 { DAY + 1 } else { 6 * HOUR }, kind) == 0);
            lab.wait(60_000, kind);
            lab.run(60_000);
            let held = lab.held();
            most = (most.0.max(held.0), most.1.max(held.1));
        }
        lab.idle(DAY + 1);
        assert_eq!((silent, lab.held()), (0, (0, 0)), "{kind:?}");
        assert!(most.0 <= 4 && most.1 <= 400, "{kind:?} {most:?}");
        let mut lab = Lab::new();
        let mut silent = 0;
        for _ in 0..3000 {
            silent += usize::from(lab.wait(6_000, kind) == 0);
            lab.run(6_000);
        }
        let peak = lab.held();
        lab.idle(DAY + 1);
        println!(
            "{kind:?}: 3,000 flaps hold {} counts and {} bytes of names",
            peak.0, peak.1
        );
        assert_eq!((silent, lab.held()), (0, (0, 0)), "{kind:?}");
        assert!(peak.1 < 200_000, "{kind:?} {peak:?}");
    }
}

/// A request's wait with no card ends; its card comes within the hour and
/// stays 26 hours; the card ends and the request waits with no card again.
/// The card is another wait than the one that ended, and sounds (the
/// verifier wanted it silent, as at the base; an extra ping at most).
#[test]
fn a_long_card_between_two_waits_of_its_request() {
    for others in [0, 40] {
        let mut lab = Lab::new();
        let first = lab.wait(1, Kind::Named);
        lab.run(100);
        let card = lab.wait(HOUR, Kind::Card);
        let mut extra = lab.see(100, Some(row(Kind::Card, None)), Kind::Card, others, true);
        extra += (0..26).map(|_| lab.wait(HOUR, Kind::Card)).sum::<usize>();
        lab.run(100);
        let after = lab.wait(100, Kind::Named);
        assert_eq!((first, card, extra, after), (1, 1, 0, 1), "{others}");
    }
}

#[test]
fn a_card_after_two_waits_of_its_request_goes_on_once() {
    let mut lab = Lab::new();
    let first = lab.wait(1, Kind::Named);
    lab.run(100);
    let second = lab.wait(100, Kind::Named);
    lab.run(100);
    lab.see(DAY + 1, None, Kind::Named, 40, true);
    let card = lab.wait(1_000, Kind::Card);
    let on: Vec<usize> = (0..5).map(|_| lab.wait(HOUR, Kind::Card)).collect();
    lab.run(100);
    let after = lab.wait(100, Kind::Named);
    assert_eq!((first, second, card, on, after), (1, 1, 1, vec![0; 5], 1));
}
