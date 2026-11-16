//! Entries, routes and machines: one owner for each machine, and a
//! machine that goes on, with its cards as they were, when the way to it
//! changes.
use super::{fleet, here, pick, ssh, until, Fleet, HERE, INSTALL};
use crate::clients::hermes::{
    machines::Gate,
    tests_unix::{
        fake::{Fake, Hold},
        marked,
    },
};
use agent_hud_agents::hermes::AGENT;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

/// One install behind two entries, `Mini` first and `Mini over VPN`
/// second, each by a name of its own for the one host.
pub(super) fn aliased() -> Option<(Fleet, Fake, String)> {
    let mut fleet = fleet(2)?;
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[
        here(),
        ssh("a", "Mini", "m0"),
        ssh("b", "Mini over VPN", "m0b"),
    ]);
    fleet.start();
    // The first entry in the list's order owns, whichever verified
    // first: where the second did, the first takes the machine from it.
    let key = fleet.connected("Mini");
    fleet.stands("Mini over VPN", "same-machine");
    let owned = |fleet: &Fleet| fleet.book().far()[0].1.as_deref() == Some("a");
    until("the first entry owns the machine", || owned(&fleet));
    Some((fleet, gateway, key))
}

/// What a card must keep across a handover: its request, the time it
/// came, the hold that runs from it.
pub(super) fn kept(fleet: &Fleet, card: &Value) -> Value {
    json!([card["id"], card["createdAt"], fleet.times(card)])
}

/// The readers core holds for Hermes, by name.
fn readers(fleet: &Fleet, names: &[String]) -> Vec<bool> {
    let states = fleet.requests.native_states();
    names
        .iter()
        .map(|name| states.of(AGENT.id, name).is_some())
        .collect()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_alias_follows_nothing() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    assert_eq!(card["machine"], "Mini", "the owner's label");
    // One machine, owned by the first entry in the list's order.
    let number = fleet.number();
    assert_eq!(
        fleet.book().far(),
        [(key.clone(), Some("a".into()), number)]
    );
    let alias = fleet.stands("Mini over VPN", "same-machine");
    assert_eq!(alias["note"], "Mini", "same machine as that entry");
    assert_ne!(alias["id"], key.as_str());
    let polled = gateway.world().calls("session.active_list").len();
    fleet.looked(8);
    until("the gateway is polled some more", || {
        gateway.world().calls("session.active_list").len() >= polled + 6
    });
    // One set of cards and one reader, and nothing forwarded for the alias.
    assert_eq!(fleet.cards().len(), 1);
    assert_eq!(fleet.number(), number, "no route change");
    let waiting = &fleet.requests.native_waiting_sessions()[AGENT.id];
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    let alias = fleet.book().route_of("b").unwrap();
    assert!(alias.followers().is_none());
    let said = gateway.world().calls("session.active_list");
    let sockets: std::collections::BTreeSet<usize> =
        said[polled..].iter().map(|(socket, _)| *socket).collect();
    assert_eq!(sockets.len(), 1, "one follower polls the gateway");
    println!("machines with an alias: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn two_homes_on_one_host_are_two_machines() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    // One install, copied to two homes on this one host.
    let first = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let second = fleet.hosts[1].gateway("instance-2", INSTALL[0]);
    fleet.write(&[
        here(),
        ssh("a", "Mini", "m0"),
        ssh("b", "Mini, other home", "m1"),
    ]);
    fleet.start();
    let keys = [fleet.connected("Mini"), fleet.connected("Mini, other home")];
    assert_ne!(keys[0], keys[1], "the root home is part of the key");
    assert_eq!(fleet.book().far().len(), 2);
    first.asks("default", "s-one", "p-one");
    second.asks("default", "s-one", "p-one");
    until("a card for each", || fleet.cards().len() == 2);
    let mut sessions: Vec<String> = (fleet.cards().iter())
        .map(|card| card["sessionId"].as_str().unwrap().to_owned())
        .collect();
    sessions.sort();
    let mut expected: Vec<String> = (keys.iter())
        .map(|key| format!("hermes:{key}/default:s-one"))
        .collect();
    expected.sort();
    assert_eq!(sessions, expected);
    println!(
        "machines with two homes on one host: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_that_reaches_this_mac_adds_nothing() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    // This machine's Hermes is the one the entry reaches over SSH.
    fleet.local = fleet.hosts[0].root.clone();
    std::fs::write(fleet.local.join("install_id"), format!("{}\n", INSTALL[0])).unwrap();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Me over SSH", "m0")]);
    fleet.start();
    let row = fleet.stands("Me over SSH", "same-machine");
    assert_eq!(row["note"], HERE, "same machine as this one");
    assert!(row.get("here").is_none(), "{row}");
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    fleet.looked(8);
    // No far machine, and the one card is this machine's own.
    assert!(fleet.book().far().is_empty());
    assert_eq!(fleet.cards(), std::slice::from_ref(&card));
    assert_eq!(card["sessionId"], "hermes:s-one");
    assert!(card.get("machine").is_none(), "{card}");
    println!(
        "machines with an entry that is this Mac: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn owner_removed_with_a_dismissed_card() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks("default", "s-shown", "p-shown");
    gateway.asks("default", "s-gone", "p-gone");
    until("two cards show", || fleet.cards().len() == 2);
    let of = |fleet: &Fleet, session: &str| {
        let its = |card: &Value| card["sessionId"].as_str().unwrap().ends_with(session);
        fleet.cards().into_iter().find(its)
    };
    let gone = of(&fleet, "s-gone").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (fleet.requests)
        .answer(&gone, &json!({"kind": "leave"}), crate::hooks::now_ms())
        .unwrap();
    let shown = fleet.card();
    assert_eq!(shown["machine"], "Mini", "the label of the entry that owns");
    let id = shown["id"].as_str().unwrap().to_owned();
    let (before, sounds) = (kept(&fleet, &shown), fleet.sounds());
    assert_eq!(sounds.0, 1, "the two questions sounded once");
    let reader = vec![format!("{key}/default")];
    assert_eq!(readers(&fleet, &reader), [true]);
    let number = fleet.number();

    // The primary entry is removed, and the alias remains.
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0b")]);
    assert_eq!(fleet.connected("Mini over VPN"), key, "the same machine");
    assert!(fleet.row("Mini").is_none());
    let after = fleet.reached();
    // The same requests: the one that shows has the time it came and its
    // hold, and now says the label its machine goes by.
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(after["machine"], "Mini over VPN");
    assert_eq!(
        fleet.cards().len(),
        1,
        "the dismissed one is still dismissed"
    );
    // Neither was registered again, and neither was settled.
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.calls(&gone), (1, 0));
    assert_eq!(fleet.sounds(), sounds, "no alert of either kind");
    assert_eq!(
        fleet.book().far(),
        [(key.clone(), Some("b".into()), number + 1)]
    );
    assert_eq!(readers(&fleet, &reader), [true], "the reader never ended");
    until("one master is left", || fleet.masters().len() == 1);
    // The card is answered over the new owner's route.
    (fleet.requests)
        .answer(&id, &pick(&after, "one"), crate::hooks::now_ms())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("clarify.respond").is_empty()
    });
    until("the card goes", || fleet.cards().is_empty());
    assert_eq!(fleet.sounds().0, 1);
    println!(
        "machines after the owner was removed: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn owner_removed_with_a_nearly_expired_card() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (_, deadline) = fleet.times(&card);
    let (sounds, number) = (fleet.sounds(), fleet.number());

    // The handover is held where the alias says who it is again.
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("profiles.list", held.clone()));
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0b")]);
    until("the alias is asked who it is", || {
        held.entered.load(Ordering::Acquire)
    });
    until("the card is marked", || fleet.cards().iter().all(marked));
    assert_eq!(
        fleet.book().far(),
        [(key.clone(), None, number + 1)],
        "no owner yet"
    );
    // The card's own hold ends during the handover, on the deadline it
    // always had: there one millisecond before it, gone at it.
    assert_eq!(fleet.requests.deadline(&id), Some(deadline));
    assert_eq!(fleet.requests.views(deadline - 1).len(), 1);
    assert!(fleet.requests.views(deadline).is_empty());

    held.released.store(true, Ordering::Release);
    assert_eq!(fleet.connected("Mini over VPN"), key);
    // The request still waits in Hermes, and the polls of the new owner
    // find it: it is not shown again.
    let polled = gateway.world().calls("session.active_list").len();
    until("the new owner polls", || {
        gateway.world().calls("session.active_list").len() >= polled + 4
    });
    assert!(fleet.cards().is_empty(), "{:?}", fleet.cards());
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn same_machine_new_address() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (before, sounds) = (kept(&fleet, &card), fleet.sounds());
    assert_eq!(fleet.book().far(), [(key.clone(), Some("a".into()), 1)]);

    // The entry's host is another name for the same install.
    fleet.write(&[here(), ssh("a", "Mini", "m0b")]);
    until("the route changed", || {
        fleet.book().far() == [(key.clone(), Some("a".into()), 2)]
    });
    // One machine before and after: the same key, the same owner, and
    // its card as it was.
    let after = fleet.reached();
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
    assert_eq!(fleet.connected("Mini"), key);
    assert_eq!(fleet.rows().len(), 2);
    until("one master is left", || fleet.masters().len() == 1);
    let log = fleet.log();
    assert!(
        log.contains(&format!("route change machine={key} number=2")),
        "{log}"
    );
    assert!(!log.contains("retired"), "{log}");
    (fleet.requests)
        .answer(&id, &pick(&after, "two"), crate::hooks::now_ms())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("clarify.respond").is_empty()
    });
    println!("machines at the new address: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn one_alias_retargeted_while_the_other_follows() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let before = kept(&fleet, &card);
    let route = fleet.book().route_of("a").unwrap();
    let link = route.route.link().map(|(_, number)| number);
    let (said, number) = (fleet.log().len(), fleet.number());

    // The second entry is pointed at another install.
    fleet.write(&[
        here(),
        ssh("a", "Mini", "m0"),
        ssh("b", "Mini over VPN", "m1"),
    ]);
    let second = fleet.connected("Mini over VPN");
    assert_ne!(second, key, "a machine of its own");
    other.asks("default", "s-two", "p-two");
    until("a card for each", || fleet.cards().len() == 2);
    // The first machine is untouched: no mark, no reconnect.
    let first = (fleet.cards().into_iter())
        .find(|shown| shown["id"] == card["id"])
        .unwrap();
    assert_eq!(first, card, "the very same card");
    assert_eq!(kept(&fleet, &first), before);
    let now = fleet.book().route_of("a").unwrap();
    assert!(Arc::ptr_eq(&now, &route));
    assert_eq!(now.route.link().map(|(_, number)| number), link);
    let log = fleet.log()[said..].to_owned();
    assert!(!log.contains("unreachable id="), "{log}");
    assert!(!log.contains("route change"), "{log}");
    assert!(!log.contains("far m0 link"), "{log}");
    let mut far = fleet.book().far();
    far.sort();
    let mut expected = vec![
        (key, Some("a".to_owned()), number),
        (second, Some("b".to_owned()), 1),
    ];
    expected.sort();
    assert_eq!(far, expected);
    println!(
        "machines after one alias was retargeted: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn retargeted_to_another_install() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();

    // A poll is in flight: its list is held, and a second question is
    // there, in the session it is about to read, for it to find.
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", held.clone()));
    until("a poll is in flight", || {
        held.entered.load(Ordering::Acquire)
    });
    gateway.asks_quietly("default", "s-one", "p-late");
    let read = gateway.world().calls("session.events.since").len();
    // The only entry of the machine is pointed elsewhere. The old route
    // is kept from being torn down, so the poll can run to its end.
    let gate = Arc::new(Gate::default());
    *fleet.book().teardown.lock().unwrap() = Some(gate.clone());
    fleet.write(&[here(), ssh("a", "Mini", "m1")]);
    until("the list is taken", || gate.entered.load(Ordering::Acquire));
    assert_eq!(fleet.book().far(), [(key.clone(), None, 2)]);
    held.released.store(true, Ordering::Release);
    until("the poll read the session again", || {
        gateway.world().calls("session.events.since").len() > read
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    // Its result is dropped: no card for the question it found.
    assert_eq!(fleet.cards().len(), 1);
    assert!(fleet.cards().iter().all(marked));
    assert!(!fleet.log().contains("p-late"), "{}", fleet.log());

    gate.released.store(true, Ordering::Release);
    // Another machine: the old one is retired, then the new one made.
    let second = fleet.moved("Mini", &key);
    assert!(fleet.cards().is_empty(), "the old card is withdrawn");
    assert_eq!(fleet.calls(&id), (1, 1));
    assert_eq!(fleet.book().far(), [(second.clone(), Some("a".into()), 1)]);
    let log = fleet.log();
    let (retired, made) = (
        log.find(&format!("retired machine={key}"))
            .expect("retired"),
        log.find(&format!("followed machine={second}"))
            .expect("made"),
    );
    assert!(
        retired < made,
        "retired before the new machine is made: {log}"
    );
    // What the new machine asks goes by its own key; the old machine's
    // session keeps the id it had, and is not this one's.
    other.asks("default", "s-one", "p-one");
    let new = fleet.card();
    assert_eq!(new["sessionId"], format!("hermes:{second}/default:s-one"));
    assert_eq!(card["sessionId"], format!("hermes:{key}/default:s-one"));
    assert!(!marked(&new));
    until("one master is left", || fleet.masters().len() == 1);
    println!("machines after the retarget: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn same_address_other_install_behind_it() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    let id = fleet.card()["id"].as_str().unwrap().to_owned();

    // The same address, and another install behind it.
    gateway.kill();
    let other = fleet.hosts[0].gateway("instance-2", INSTALL[1]);
    until("the old machine is retired", || {
        fleet.log().contains(&format!("retired machine={key}"))
    });
    until("the old card is withdrawn", || fleet.cards().is_empty());
    assert_eq!(fleet.requests.deadline(&id), None, "it is another machine");
    let second = fleet.moved("Mini", &key);
    other.asks("default", "s-one", "p-one");
    let card = fleet.card();
    assert_eq!(card["sessionId"], format!("hermes:{second}/default:s-one"));
    assert_ne!(card["id"], id.as_str());
    assert_eq!(fleet.book().far().len(), 1);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_route_that_cannot_be_reached_retires_nothing() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (before, sounds) = (kept(&fleet, &card), fleet.sounds());

    // The only entry is pointed at an address nothing answers at: that
    // is not another machine.
    fleet.write(&[here(), ssh("a", "Mini", "gone")]);
    let row = fleet.stands("Mini", "cannot-reach");
    assert_eq!(row["id"], "a", "not verified at this address");
    until("the card is marked", || fleet.cards().iter().all(marked));
    fleet.looked(12);
    std::thread::sleep(std::time::Duration::from_secs(2));
    // The machine waits, with its card kept and marked, and is not
    // retired however long the route stays out of reach.
    assert_eq!(fleet.book().far(), [(key.clone(), None, 2)]);
    let waiting = fleet.card();
    assert!(marked(&waiting));
    assert_eq!(kept(&fleet, &waiting), before);
    assert!(!fleet.log().contains("retired"), "{}", fleet.log());
    let refused = fleet
        .requests
        .answer(&id, &pick(&card, "one"), crate::hooks::now_ms());
    assert!(refused.is_err(), "a marked card took an answer");

    // Pointed back at the machine: it goes on, with the same request.
    // The machine had no way while it waited, so its number is as the
    // one route change left it.
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    until("the machine has its owner again", || {
        fleet.book().far() == [(key.clone(), Some("a".into()), 2)]
    });
    let after = fleet.reached();
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn retargeted_to_a_machine_whose_identity_is_refused_retires_the_old_one() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    // The other host's Hermes reports no install identity.
    let _other = fleet.hosts[1].gateway("instance-2", "");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    let id = fleet.card()["id"].as_str().unwrap().to_owned();

    fleet.write(&[here(), ssh("a", "Mini", "m1")]);
    // Reached and refused: the entry leads to that machine no more, and
    // no other entry does, so it is retired.
    until("the old machine is retired", || {
        fleet.log().contains(&format!("retired machine={key}"))
    });
    assert!(fleet.cards().is_empty(), "its card is withdrawn");
    assert_eq!(fleet.calls(&id), (1, 1));
    assert!(fleet.book().far().is_empty());
    let row = fleet.stands("Mini", "not-covered");
    assert_eq!(row["id"], "a");
    until("one master is left", || fleet.masters().len() == 1);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_stopped_gateway_behind_a_new_address_retires_nothing() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let before = kept(&fleet, &card);

    // The gateway stops, and then the entry's address is edited: its new
    // route reaches a machine where nothing can say who it is. That is
    // not word of another machine.
    gateway.kill();
    until("the card is marked", || fleet.cards().iter().all(marked));
    fleet.write(&[here(), ssh("a", "Mini", "m0b")]);
    fleet.stands("Mini", "not-covered");
    fleet.looked(12);
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(fleet.book().far(), [(key.clone(), None, 2)]);
    assert_eq!(kept(&fleet, &fleet.card()), before);
    assert!(!fleet.log().contains("retired"), "{}", fleet.log());

    // The same gateway is back, with its question still waiting.
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    gateway.asks("default", "s-one", "p-one");
    assert_eq!(fleet.connected("Mini"), key);
    let after = fleet.reached();
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(fleet.calls(&id), (1, 0));
}
