//! A wait whose kind changes while it goes on (Agent HUD Fork, ticket 81,
//! from ticket 75's round 3 verifier): its request shows a card, loses it,
//! or stops being listed as an ask, and the session waits throughout. Each
//! look's pings are what the host plays (`lab.rs`).
use super::lab::*;
use super::T;

/// A request waits with no card `earlier` times, each ending; then it
/// waits with no card again (`told`), its card comes, and it goes on with
/// no card. Then it waits once more after an end.
fn card_inside(told: Kind, card: Kind, earlier: usize) -> (Vec<usize>, Vec<usize>, usize) {
    let mut lab = Lab::new();
    let mut before = Vec::new();
    for _ in 0..earlier {
        before.push(lab.wait(1000, told));
        lab.run(1000);
    }
    let wait = vec![
        lab.wait(1000, told),
        lab.wait(1000, card),
        lab.wait(1000, told),
    ];
    lab.run(1000);
    (before, wait, lab.wait(1000, told))
}

#[test]
fn a_card_that_comes_inside_a_later_wait_of_its_request_with_no_card_pings_once() {
    for (told, card) in [(Kind::Named, Kind::Card), (Kind::Ask, Kind::CardAsk)] {
        for earlier in 0..3 {
            let (before, wait, next) = card_inside(told, card, earlier);
            let case = format!("{told:?} then {card:?}, {earlier} earlier waits");
            assert_eq!(before, vec![1; earlier], "{case}");
            assert_eq!(wait, [1, 0, 0], "{case}");
            assert_eq!(next, 1, "{case}");
        }
    }
}

#[test]
fn a_request_that_goes_on_does_not_ping_when_its_ask_stops_being_listed() {
    // Asked, then named alone, then named alone; and back. A card with its
    // ask the same way. Each goes on as one wait, and the next one sounds.
    let cases = [
        [Kind::Ask, Kind::Named, Kind::Named, Kind::Ask],
        [Kind::CardAsk, Kind::Named, Kind::Named, Kind::CardAsk],
        [Kind::Named, Kind::Ask, Kind::Named, Kind::Named],
        [Kind::Ask, Kind::Asked, Kind::Asked, Kind::Ask],
        [Kind::Asked, Kind::Ask, Kind::Named, Kind::Ask],
    ];
    for kinds in cases {
        for earlier in 0..2 {
            let mut lab = Lab::new();
            for _ in 0..earlier {
                lab.wait(1000, kinds[0]);
                lab.run(1000);
            }
            let pings: Vec<usize> = kinds.iter().map(|k| lab.wait(1000, *k)).collect();
            let case = format!("{kinds:?}, {earlier} earlier waits");
            // After an earlier wait, a card of the request is silent within
            // the day (ticket 66), and the look with no card is the wait's
            // one ping, as on the base.
            let carded_again = earlier > 0 && kinds[0] == Kind::CardAsk;
            let wanted = if carded_again {
                [0, 1, 0, 0]
            } else {
                [1, 0, 0, 0]
            };
            assert_eq!(pings, wanted, "{case}");
            assert_eq!(lab.run(1000), 0, "{case}");
            // A card shown again within the day is the same card (ticket 66).
            let next = usize::from(kinds[0] != Kind::CardAsk);
            assert_eq!(lab.wait(1000, kinds[0]), next, "{case}: the next wait");
        }
    }
}

/// A request waits with no card, as an ask and then named alone, and the
/// app quits while it waits. After the relaunch the session is not listed
/// at first; then its card comes. That is another wait, and it sounds, as
/// on the base (the probe `k81`, life 329 of `changing`).
#[test]
fn a_card_after_a_relaunch_that_ended_a_wait_of_its_request_with_no_card_sounds() {
    let folder = tempfile::tempdir().unwrap();
    for (case, kinds) in [
        ("asked, then named", &[Kind::Ask, Kind::Named][..]),
        ("named", &[Kind::Named]),
        ("asked", &[Kind::Ask]),
        ("asked only", &[Kind::Asked]),
    ] {
        for card in [Kind::Card, Kind::CardAsk] {
            let file = folder.path().join(format!("{case}-{card:?}.json"));
            let mut lab = Lab::kept(&file, T);
            lab.idle(0);
            let pings: Vec<usize> = kinds.iter().map(|k| lab.wait(1000, *k)).collect();
            assert_eq!(pings[0], 1, "{case}, {card:?}");
            let mut lab = Lab::kept(&file, lab.now + 60_000);
            assert_eq!(lab.run(1000), 0, "{case}, {card:?}");
            assert_eq!(lab.wait(1000, card), 1, "{case}, {card:?}: the card");
            assert_eq!(lab.wait(1000, card), 0, "{case}, {card:?}: it goes on");
        }
    }
}

/// A card from a client that does not name its request for the wait, and
/// it ends; the request then waits with no card within the day. That is
/// another wait, and it sounds; the card shown again is the same card
/// (ticket 66). Before, the wait with no card went by the name the card
/// had decided and was silent, on the base too (`5. 0. 2.`).
#[test]
fn a_wait_with_no_card_after_a_card_its_client_did_not_name_sounds() {
    for told in [Kind::Named, Kind::Ask, Kind::Asked] {
        let mut lab = Lab::new();
        assert_eq!(lab.wait(1000, Kind::CardOnly), 1, "{told:?}");
        lab.run(1000);
        assert_eq!(lab.wait(1000, told), 1, "{told:?}: the wait with no card");
        assert_eq!(lab.wait(1000, Kind::Card), 0, "{told:?}: its card in it");
        lab.run(1000);
        assert_eq!(
            lab.wait(1000, Kind::CardOnly),
            0,
            "{told:?}: the card again"
        );
    }
}

/// A card, then the request with no card, then its card again, each wait
/// ending before the next, within the day. Each is a wait of its own, and
/// sounds: the wait with no card goes by a name after the card's, and once
/// it ends the card's name is forgotten (`spent`, at any count).
#[test]
fn a_card_after_its_requests_wait_with_no_card_under_a_later_name_sounds() {
    for (card, told) in [(Kind::Card, Kind::Named), (Kind::CardAsk, Kind::Ask)] {
        let mut lab = Lab::new();
        let mut pings = Vec::new();
        for kind in [card, told, card, told] {
            pings.push(lab.wait(1000, kind));
            lab.run(1000);
        }
        assert_eq!(pings, [1, 1, 1, 1], "{card:?} and {told:?}");
    }
}
