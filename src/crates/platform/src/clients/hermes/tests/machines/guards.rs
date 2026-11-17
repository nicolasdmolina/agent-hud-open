//! What ticket 32's review left (ticket 52): one status for each machine
//! key, the safeguards that had no test of their own, and two edits that
//! arrive in one look.
use super::{
    fleet, here,
    lists::one_machine,
    owners::{aliased, kept},
    ssh, text, until, Fleet, INSTALL,
};
use crate::clients::hermes::tests_unix::marked;
use serde_json::{json, Value};

/// The id each status goes by, in the list's order.
fn ids(fleet: &mut Fleet) -> Vec<String> {
    let id = |row: &Value| row["id"].as_str().unwrap().to_owned();
    fleet.rows().iter().map(id).collect()
}

/// The far host goes away: its `sshd`, and every login it held.
fn offline(fleet: &mut Fleet) {
    let far = fleet.hosts.remove(0);
    let port = far.lab.port;
    drop(far);
    until("the sshd is gone", || {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
    });
}

/// The ports the master of the entry `entry` listens on: its forwards.
fn listening(fleet: &Fleet, entry: &str) -> Vec<String> {
    let route = fleet.book().route_of(entry).expect("its route");
    let Some((link, _)) = route.route.link() else {
        return vec!["no link".into()];
    };
    let pid = link.master_pid().expect("its master").to_string();
    let listed = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fn"])
        .output()
        .expect("lsof");
    let said = String::from_utf8_lossy(&listed.stdout).into_owned();
    said.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn two_entries_of_one_machine_out_of_reach_go_by_two_ids() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    assert_eq!(ids(&mut fleet), ["local", key.as_str(), "b"]);
    // Both entries verified, so both are bound to the machine's key.
    let saved = std::fs::read_to_string(fleet.data.join("hermes-machines.json")).unwrap();
    assert_eq!(saved.matches(key.as_str()).count(), 2, "{saved}");
    // A relaunch with the far host offline: neither entry is reached.
    fleet.stop();
    gateway.kill();
    offline(&mut fleet);
    fleet.start();
    fleet.stands("Mini", "cannot-reach");
    fleet.stands("Mini over VPN", "cannot-reach");
    // One status goes by the machine's key, the first entry's; the other
    // entry that leads there goes by its own id, as an alias does.
    assert_eq!(ids(&mut fleet), ["local", key.as_str(), "b"]);
    fleet.looked(4);
    assert_eq!(ids(&mut fleet), ["local", key.as_str(), "b"]);
    println!("machines with two entries offline: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_out_of_reach_leaves_the_machines_key_to_its_owner() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let _gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini over VPN");
    fleet.stop();
    // A binding made for the test: the entry `a`, which comes first in
    // the list, verified as the same machine by a name that nothing
    // answers at now.
    let bound = json!({"version": 1, "bindings": [
        {"entry": "a", "host": "gone", "key": key, "label": "Mini"},
        {"entry": "b", "host": "m0", "key": key, "label": "Mini over VPN"}]});
    std::fs::write(fleet.data.join("hermes-machines.json"), bound.to_string()).unwrap();
    fleet.write(&[
        here(),
        ssh("a", "Mini", "gone"),
        ssh("b", "Mini over VPN", "m0"),
    ]);
    fleet.start();
    assert_eq!(fleet.connected("Mini over VPN"), key);
    let first = fleet.stands("Mini", "cannot-reach");
    // The later entry owns the machine and goes by its key; the earlier
    // one, out of reach, does not take the key beside it.
    assert_eq!(first["id"], "a", "{first}");
    assert_eq!(ids(&mut fleet), ["local", "a", key.as_str()]);
    assert_eq!(fleet.book().far(), [(key.clone(), Some("b".into()), 1)]);
    println!(
        "machines with the first entry offline: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_list_taken_after_the_stop_starts_nothing() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let _gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here()]);
    fleet.start();
    fleet.looked(2);
    // The look that read a list with a new entry was under way when
    // client hooks turned off: the list reaches a book that has stopped.
    let book = fleet.book().clone();
    fleet.stop();
    let list = text(&[here(), ssh("a", "Mini", "m0")], "token-1");
    book.apply(agent_hud_agents::hermes::connections::parse(&list).unwrap());
    let route = book.route_of("a");
    // Closed first where one did start, so a failure leaves no process.
    if let Some(route) = &route {
        route.stop();
    }
    assert!(route.is_none(), "a route started after the stop");
    assert!(fleet.masters().is_empty(), "{:?}", fleet.masters());
    assert!(fleet.rows().is_empty(), "machines told after the stop");
    assert_eq!(fleet.knocks(0), 0, "nothing reached the sshd");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_second_entry_of_one_id_is_never_dialled() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let _first = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let _second = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("a", "Twin", "m1")]);
    fleet.start();
    let key = fleet.connected("Mini");
    // The first entry of the id is the entry: the second has no status,
    // no route and no machine.
    assert_eq!(ids(&mut fleet), ["local", key.as_str()]);
    fleet.looked(8);
    assert_eq!(ids(&mut fleet), ["local", key.as_str()]);
    assert_eq!(fleet.book().far(), [(key.clone(), Some("a".into()), 1)]);
    assert_eq!(fleet.masters().len(), 1, "{:?}", fleet.masters());
    assert_eq!(fleet.knocks(1), 0, "the second host was dialled");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_that_left_the_list_is_forgotten() {
    let Some((mut fleet, _gateway, key)) = one_machine() else {
        return;
    };
    let file = fleet.data.join("hermes-machines.json");
    assert!(std::fs::read_to_string(&file).unwrap().contains(&key));
    fleet.write(&[here()]);
    until("the card is withdrawn", || fleet.cards().is_empty());
    until("no ssh process is left", || fleet.masters().is_empty());
    // Its binding went with it: nothing of the machine is in the file.
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved, json!({"version": 1, "bindings": []}));
    // So a launch with no list that parses does not show it again.
    fleet.stop();
    fleet.write_text("{\"version\": 2, \"connections\": [{\"id\": \"a\", \"kind\":");
    fleet.start();
    fleet.looked(4);
    let rows = fleet.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["here"], true);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_reordered_list_gives_the_machine_to_the_entry_now_first() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (before, sounds, number) = (kept(&fleet, &card), fleet.sounds(), fleet.number());
    // The same two entries, the alias first now.
    fleet.write(&[
        here(),
        ssh("b", "Mini over VPN", "m0b"),
        ssh("a", "Mini", "m0"),
    ]);
    // Watched all through the handover: for a moment both routes say
    // they are connected to the machine, and still no two statuses go by
    // one id.
    let mut twice = Vec::new();
    until("the entry that comes first owns the machine", || {
        let mut seen = ids(&mut fleet);
        seen.sort();
        if seen.windows(2).any(|pair| pair[0] == pair[1]) {
            twice.push(seen);
        }
        let owned = fleet.book().far() == [(key.clone(), Some("b".into()), number + 1)];
        owned
            && fleet
                .row("Mini")
                .is_some_and(|row| row["state"] == "same-machine")
    });
    assert!(twice.is_empty(), "two statuses went by one id: {twice:?}");
    // A handover like any other: the same request, nothing registered
    // again, no sound, and the card under the new owner's label.
    let after = fleet.reached();
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(after["machine"], "Mini over VPN");
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
    fleet.stands("Mini", "same-machine");
    assert_eq!(ids(&mut fleet), ["local", key.as_str(), "a"]);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_alias_keeps_no_forward() {
    let Some((fleet, gateway, _key)) = aliased() else {
        return;
    };
    gateway.asks("default", "s-one", "p-one");
    fleet.card();
    // The owner's master holds the one forward to the one gateway, so a
    // forward is there to be counted.
    until("the owner holds one forward", || {
        listening(&fleet, "a").len() == 1
    });
    // Verifying made the alias a forward too. It is let go.
    let alias = fleet.book().route_of("b").expect("the alias's route");
    assert!(alias.route.forwards_made() >= 1, "verifying forwards");
    until("the alias holds no forward", || {
        listening(&fleet, "b").is_empty()
    });
    fleet.looked(8);
    assert_eq!(listening(&fleet, "b"), Vec::<String>::new());
    assert_eq!(listening(&fleet, "a").len(), 1);
    assert_eq!(fleet.masters().len(), 2, "the alias keeps its link");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_removed_while_its_machine_waits_on_it_retires_the_machine() {
    let Some((fleet, _gateway, key)) = one_machine() else {
        return;
    };
    let id = fleet.card()["id"].as_str().unwrap().to_owned();
    // Its address is edited to one nothing answers at: the machine
    // waits on the entry's new route, its card marked.
    fleet.write(&[here(), ssh("a", "Mini", "gone")]);
    until("the machine waits", || {
        fleet.book().far() == [(key.clone(), None, 2)]
    });
    until("the card is marked", || fleet.cards().iter().all(marked));
    assert_eq!(fleet.cards().len(), 1);
    // Then the entry is removed: no entry is left that may lead there.
    fleet.write(&[here()]);
    until("the machine is retired", || fleet.book().far().is_empty());
    until("the card is withdrawn", || fleet.cards().is_empty());
    until("no ssh process is left", || fleet.masters().is_empty());
    assert!(fleet.log().contains(&format!("retired machine={key}")));
    assert_eq!(fleet.calls(&id), (1, 1), "withdrawn once");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn owner_removed_and_the_alias_readdressed_in_one_look() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (before, sounds, number) = (kept(&fleet, &card), fleet.sounds(), fleet.number());
    // One list: the owner is gone, and the alias goes by the other name
    // of the same host.
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0")]);
    until("the alias owns the machine by its new address", || {
        fleet.book().far() == [(key.clone(), Some("b".into()), number + 1)]
    });
    assert_eq!(fleet.connected("Mini over VPN"), key);
    // The machine went on: the same card, with the time it came and its
    // hold as they were, registered once and never settled.
    let after = fleet.reached();
    assert_eq!(kept(&fleet, &after), before);
    assert_eq!(after["machine"], "Mini over VPN");
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
    assert!(!fleet.log().contains("retired machine="), "{}", fleet.log());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_alias_readdressed_out_of_reach_is_waited_on_when_the_owner_goes() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks("default", "s-one", "p-one");
    let card = fleet.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let (before, sounds, number) = (kept(&fleet, &card), fleet.sounds(), fleet.number());
    // First the alias's address is edited to one nothing answers at.
    // The machine and its card are untouched.
    let away = ssh("b", "Mini over VPN", "gone");
    fleet.write(&[here(), ssh("a", "Mini", "m0"), away.clone()]);
    fleet.stands("Mini over VPN", "cannot-reach");
    assert_eq!(
        fleet.book().far(),
        [(key.clone(), Some("a".into()), number)]
    );
    assert_eq!(kept(&fleet, &fleet.reached()), before);
    // Then, in a later list, the owner is removed. Nothing has said that
    // the alias leads elsewhere now, so the machine waits on it, as it
    // does where the two edits come in the other order.
    fleet.write(&[here(), away]);
    until("the machine waits", || {
        fleet.book().far() == [(key.clone(), None, number + 1)]
    });
    until("the card is marked", || fleet.cards().iter().all(marked));
    fleet.looked(8);
    assert_eq!(kept(&fleet, &fleet.card()), before);
    assert!(!fleet.log().contains("retired machine="), "{}", fleet.log());
    // Pointed at the machine again, it goes on with the same card.
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0")]);
    until("the alias owns the machine", || {
        fleet.book().far() == [(key.clone(), Some("b".into()), number + 1)]
    });
    assert_eq!(kept(&fleet, &fleet.reached()), before);
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds);
}

/// Ticket 53: a list with no entry for this Mac, and an `ssh` entry
/// whose id is the word this Mac's own status goes by.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_named_local_is_not_this_mac() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    fleet.write(&[ssh("local", "Mini", "gone")]);
    fleet.start();
    fleet.stands("Mini", "cannot-reach");
    assert_eq!(ids(&mut fleet), ["local", "entry:local"]);
    let rows = fleet.rows();
    assert_eq!(rows[0]["here"], true, "this Mac keeps its own id");
    assert!(rows[1].get("here").is_none(), "{}", rows[1]);
}
