//! Answers and a route change: none is sent across one, and an answer
//! that fails late brings back only what is still the machine's.
use super::{fleet, here, pick, ssh, until, Fleet, INSTALL};
use crate::clients::hermes::{
    machines::Gate,
    tests_unix::{
        fake::{Fake, Hold},
        marked,
    },
};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

fn now() -> i64 {
    crate::hooks::now_ms()
}

/// One far machine behind the entry `Mini`, with one question showing,
/// and a second host that is another install. The first look at core is
/// taken, so what sounds from here on is counted.
fn asked() -> Option<(Fleet, Fake, Fake, String, Value)> {
    let mut fleet = fleet(2)?;
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    Some((fleet, gateway, other, key, card))
}

/// Accepts an answer to `card` and holds its send back: the gateway
/// keeps its reply to the subscription the answer needs.
fn held_send(fleet: &Fleet, gateway: &Fake, card: &Value) -> Arc<Hold> {
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.resume", held.clone()));
    let id = card["id"].as_str().unwrap();
    fleet
        .requests
        .answer(id, &pick(card, "one"), now())
        .unwrap();
    until("the send is held back", || {
        held.entered.load(Ordering::Acquire)
    });
    assert!(fleet.cards().is_empty(), "core put the card away");
    held
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn no_answer_crosses_a_route_change() {
    let Some((mut fleet, gateway, other, key, first)) = asked() else {
        return;
    };
    gateway.asks("default", "s-two", "p-two");
    until("two cards show", || fleet.cards().len() == 2);
    let second = (fleet.cards().into_iter())
        .find(|card| card["id"] != first["id"])
        .unwrap();
    let (one, two) = (
        first["id"].as_str().unwrap().to_owned(),
        second["id"].as_str().unwrap().to_owned(),
    );

    // A click just before: core has taken the decision and not yet
    // handed it over when the route number goes up.
    let taken = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    (fleet.requests).reviewer27_hold_acceptance(Some((one.clone(), taken.clone())));
    let (requests, clicked, decision) = (fleet.requests.clone(), one.clone(), pick(&first, "one"));
    let click = std::thread::spawn(move || requests.answer(&clicked, &decision, now()));
    until("core took the decision", || taken.0.load(Ordering::Acquire));

    // The only entry is pointed at another install. The list is held
    // where its locked step is over and no route is torn down yet. The
    // new route's gateway is held where it says who it is: told, the old
    // machine is retired, and its waiter for the decision goes with it.
    // On a loaded machine that came before the decision was handed over
    // (ticket 59), which then found nobody waiting.
    let verify = Arc::new(Hold::default());
    other.world().hold_next = Some(("profiles.list", verify.clone()));
    let gate = Arc::new(Gate::default());
    *fleet.book().teardown.lock().unwrap() = Some(gate.clone());
    fleet.write(&[here(), ssh("a", "Mini", "m1")]);
    until("the route number went up", || {
        gate.entered.load(Ordering::Acquire)
    });
    assert_eq!(fleet.book().far()[0], (key.clone(), None, 2));
    let sockets = gateway.world().sockets;
    let old_link = fleet.masters().len();
    assert!(old_link >= 1, "the old route is not torn down yet");

    // A click after the number went up, before teardown began: refused,
    // and no socket is opened for it.
    let refused = fleet.requests.answer(&two, &pick(&second, "two"), now());
    assert!(
        refused.is_err(),
        "an answer was taken after the route changed"
    );
    // The decision taken before it is handed over now, and is stale.
    taken.1.store(true, Ordering::Release);
    click.join().unwrap().unwrap();
    fleet.requests.reviewer27_hold_acceptance(None);
    until("the stale decision is dealt with", || {
        fleet.log().contains(&format!("undelivered id={one}")) || fleet.cards().is_empty()
    });
    assert_eq!(
        gateway.world().sockets,
        sockets,
        "no socket to the old machine"
    );
    assert!(gateway.world().calls("clarify.respond").is_empty());
    verify.released.store(true, Ordering::Release);

    // A click after teardown began: refused as well.
    gate.released.store(true, Ordering::Release);
    for id in [&one, &two] {
        let refused = fleet.requests.answer(id, &pick(&second, "two"), now());
        assert!(refused.is_err(), "an answer was taken during teardown");
    }
    fleet.moved("Mini", &key);
    until("the old cards are withdrawn", || fleet.cards().is_empty());
    fleet.looked(6);
    // Sent to neither machine: not the old one, and not the new one.
    for (machine, heard) in [("old", &gateway), ("new", &other)] {
        let world = heard.world();
        assert!(world.calls("clarify.respond").is_empty(), "{machine}");
        assert!(world.calls("session.resume").is_empty(), "{machine}");
    }
    assert_eq!(gateway.world().sockets, sockets);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_late_failed_answer_after_retirement_restores_nothing() {
    let Some((mut fleet, gateway, _other, key, card)) = asked() else {
        return;
    };
    let id = card["id"].as_str().unwrap().to_owned();
    let sounds = fleet.sounds();
    assert_eq!(sounds, (1, 0));
    let held = held_send(&fleet, &gateway, &card);

    // The machine's last owner is removed.
    fleet.write(&[here()]);
    until("the machine is retired", || {
        fleet.log().contains(&format!("retired machine={key}"))
    });
    until("its route is closed", || fleet.masters().is_empty());
    // Only now does the send fail.
    held.released.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(400));
    fleet.looked(5);

    assert!(
        fleet.cards().is_empty(),
        "no card of the old machine comes back"
    );
    assert_eq!(fleet.requests.deadline(&id), None);
    assert_eq!(fleet.sounds(), sounds, "nothing sounds");
    assert_eq!(fleet.calls(&id).0, 1, "never registered again");
    assert!(gateway.world().calls("clarify.respond").is_empty());
    assert!(fleet.book().far().is_empty());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_late_failed_answer_after_a_retarget_restores_nothing() {
    let Some((mut fleet, gateway, other, key, card)) = asked() else {
        return;
    };
    let id = card["id"].as_str().unwrap().to_owned();
    let held = held_send(&fleet, &gateway, &card);

    // The only entry is pointed at another install, which asks too.
    fleet.write(&[here(), ssh("a", "Mini", "m1")]);
    let new = fleet.moved("Mini", &key);
    other.asks("default", "s-new", "p-new");
    let theirs = fleet.card();
    assert_eq!(theirs["sessionId"], format!("hermes:{new}/default:s-new"));
    let (times, sounds) = (fleet.times(&theirs), fleet.sounds());
    assert_eq!(sounds, (2, 0));
    until("the old route is closed", || fleet.masters().len() == 1);
    // Only now does the send to the old machine fail.
    held.released.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(400));
    fleet.looked(5);

    // No card of the old machine, and the new machine's untouched.
    assert_eq!(
        fleet.cards(),
        std::slice::from_ref(&theirs),
        "the very same card, alone"
    );
    assert!(!marked(&theirs));
    assert_eq!(fleet.times(&theirs), times);
    assert_eq!(fleet.requests.deadline(&id), None);
    assert_eq!(fleet.sounds(), sounds, "nothing sounds");
    assert_eq!(fleet.calls(&id).0, 1);
    assert_eq!(fleet.calls(theirs["id"].as_str().unwrap()), (1, 0));
    for heard in [&gateway, &other] {
        assert!(heard.world().calls("clarify.respond").is_empty());
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_late_failed_answer_during_a_handover_is_put_back_once() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[
        here(),
        ssh("a", "Mini", "m0"),
        ssh("b", "Mini over VPN", "m0b"),
    ]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.stands("Mini over VPN", "same-machine");
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    assert_eq!(card["machine"], "Mini");
    let id = card["id"].as_str().unwrap().to_owned();
    let (times, sounds) = (fleet.times(&card), fleet.sounds());
    let held = held_send(&fleet, &gateway, &card);

    // The owner is removed while the alias remains: a handover of the
    // same machine. The request still waits, and its answer never left.
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0b")]);
    until("the card is back", || fleet.cards().len() == 1);
    let back = fleet.card();
    assert_eq!(back["id"], id.as_str());
    assert_eq!(back["createdAt"], card["createdAt"]);
    assert_eq!(fleet.times(&back), times, "the time it came and its hold");
    // Only now does the send fail.
    until("the old route is closed", || fleet.masters().len() == 1);
    held.released.store(true, Ordering::Release);

    assert_eq!(fleet.connected("Mini over VPN"), key);
    let after = fleet.reached();
    assert_eq!(after["id"], id.as_str());
    assert_eq!(after["machine"], "Mini over VPN", "under the new owner");
    assert_eq!(fleet.times(&after), times);
    std::thread::sleep(Duration::from_millis(400));
    fleet.looked(5);
    // Once: one put-back, one receipt, one request, no second sound.
    assert_eq!(fleet.cards().len(), 1);
    let log = fleet.log();
    assert_eq!(
        log.matches(&format!("undelivered id={id}")).count(),
        1,
        "{log}"
    );
    assert_eq!(fleet.requests.reviewer27_notes(&id).len(), 1);
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
    assert!(gateway.world().calls("clarify.respond").is_empty());
    // And it takes a new answer, which reaches Hermes.
    (fleet.requests)
        .answer(&id, &pick(&after, "two"), now())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("clarify.respond").is_empty()
    });
}
