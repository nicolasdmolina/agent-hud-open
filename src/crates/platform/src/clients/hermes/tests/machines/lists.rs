//! The list: what a list that parsed changes, and that nothing else
//! changes anything.
use super::{fleet, here, pick, ssh, text, until, Fleet, HERE, INSTALL, MARKER};
use crate::clients::hermes::tests_unix::{fake::Fake, marked};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;

/// A fleet whose first host runs a gateway that asks one question, with
/// the list holding this machine and the entry `Mini` for that host.
pub(super) fn one_machine() -> Option<(Fleet, Fake, String)> {
    let mut fleet = fleet(1)?;
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    fleet.card();
    Some((fleet, gateway, key))
}

/// What must stay as it is while a list changes nothing: each route's
/// link and forwards, each card with the time it came and its hold.
fn standing(fleet: &Fleet, entries: &[&str]) -> Value {
    let routes: Vec<Value> = (entries.iter())
        .map(|entry| {
            let route = fleet.book().route_of(entry).expect("its route");
            let link = route.route.link().map(|(_, number)| number);
            json!([
                entry,
                link,
                route.route.forwards_made(),
                std::sync::Arc::as_ptr(&route) as usize
            ])
        })
        .collect();
    let cards: Vec<Value> = (fleet.cards().iter())
        .map(|card| {
            json!([
                card["id"],
                card["createdAt"],
                fleet.times(card),
                marked(card)
            ])
        })
        .collect();
    json!({"routes": routes, "cards": cards, "machines": format!("{:?}", fleet.book().far())})
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn adding_an_entry_brings_its_machine_up() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    gateway.asks("default", "s-one", "p-one");
    fleet.write(&[here()]);
    fleet.start();
    fleet.looked(2);
    // This machine alone: a list of only this machine.
    let rows = fleet.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["label"], HERE);
    assert_eq!(rows[0]["here"], true);
    assert!(fleet.masters().is_empty() && fleet.cards().is_empty());

    // The entry is added while the app runs: no relaunch.
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    let key = fleet.connected("Mini");
    let card = fleet.card();
    assert_eq!(card["machine"], "Mini");
    let session = format!("hermes:{key}/default:s-one");
    assert_eq!(card["sessionId"], session, "{card}");
    assert_eq!(fleet.book().far(), [(key.clone(), Some("a".into()), 1)]);
    println!("machines after adding an entry: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn removing_the_only_entry_retires_it() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    let id = fleet.card()["id"].as_str().unwrap().to_owned();
    assert_eq!(fleet.masters().len(), 1, "one master while it is followed");
    let sockets = gateway.world().sockets;

    fleet.write(&[here()]);
    until("the card is withdrawn", || fleet.cards().is_empty());
    until("its status goes", || fleet.row("Mini").is_none());
    // Authoritative: the request is settled, its reader ended.
    assert_eq!(fleet.calls(&id), (1, 1));
    assert!(fleet.book().far().is_empty());
    let waiting = fleet.requests.native_waiting_sessions();
    let far = |session: &&String| session.contains(key.as_str());
    assert_eq!(
        waiting.values().flatten().filter(far).count(),
        0,
        "{waiting:?}"
    );
    let reader = format!("{key}/default");
    let source = agent_hud_agents::hermes::AGENT.id;
    assert!(fleet.requests.native_states().of(source, &reader).is_none());
    until("no ssh process is left", || fleet.masters().is_empty());
    // No socket and no forward: the folder holds nothing of the master.
    let left = || crate::ssh_link::unix::tests::lab::entries(&fleet.masters);
    until("the control folder is empty", || left().is_empty());
    // Nothing is asked of the machine afterwards.
    fleet.looked(5);
    assert_eq!(gateway.world().sockets, sockets);
    assert!(fleet.log().contains(&format!("retired machine={key}")));
    println!("machines after removal: {}", json!(fleet.rows()));
    println!("processes after removal: {:?}", fleet.masters());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_token_only_rewrite_changes_nothing() {
    let Some((mut fleet, gateway, _)) = one_machine() else {
        return;
    };
    let before = standing(&fleet, &["a"]);
    let asked = gateway.world().calls("profiles.list").len();
    let rows = fleet.rows();
    // The desktop app's own token changed: the file is another, the list
    // it holds is the same.
    for token in ["token-2", "token-3"] {
        fleet.write_text(&text(&[here(), ssh("a", "Mini", "m0")], token));
        fleet.looked(3);
    }
    assert_eq!(standing(&fleet, &["a"]), before, "no reconnect");
    assert_eq!(gateway.world().calls("profiles.list").len(), asked);
    assert_eq!(fleet.rows(), rows);
    let log = fleet.log();
    assert_eq!(log.matches("link up").count(), 1, "{log}");
    assert!(!log.contains("route change"), "{log}");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_unreadable_list_changes_nothing() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let first = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let second = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    let list = [here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")];
    fleet.write(&list);
    fleet.start();
    let keys = [fleet.connected("Mini"), fleet.connected("Studio")];
    // A question waits on each, and one more that is dismissed.
    first.asks("default", "s-one", "p-one");
    second.asks("default", "s-two", "p-two");
    first.asks("default", "s-gone", "p-gone");
    until("three cards show", || fleet.cards().len() == 3);
    let gone = (fleet.cards().into_iter())
        .find(|card| card["sessionId"].as_str().unwrap().ends_with("s-gone"))
        .unwrap();
    let gone = gone["id"].as_str().unwrap().to_owned();
    (fleet.requests)
        .answer(&gone, &json!({"kind": "leave"}), crate::hooks::now_ms())
        .unwrap();
    until("two cards show", || fleet.cards().len() == 2);
    let before = standing(&fleet, &["a", "b"]);
    let rows = fleet.rows();
    assert_eq!(fleet.note(), None);
    println!("machines with two far machines up: {}", json!(rows));

    let broken = format!("{{\"version\": 2, \"token\": \"{MARKER}\", \"connections\": [{{\"id\":");
    let newer = text(&list, "token-1").replace("\"version\": 2", "\"version\": 3");
    for step in ["unreadable", "broken", "version 3"] {
        match step {
            "unreadable" => {
                let closed = std::fs::Permissions::from_mode(0o000);
                std::fs::set_permissions(&fleet.list, closed).unwrap();
            }
            "broken" => fleet.write_text(&broken),
            _ => fleet.write_text(&newer),
        }
        until("the note is set", || fleet.note().is_some());
        fleet.looked(4);
        // The same routes, with no reconnect; the same requests, with
        // the times they came and their holds; the dismissed one still
        // dismissed; and the note.
        assert_eq!(standing(&fleet, &["a", "b"]), before, "after {step}");
        assert_eq!(fleet.calls(&gone), (1, 0), "after {step}");
        assert_eq!(fleet.rows(), rows, "after {step}");
        assert!(fleet.note().is_some(), "after {step}");
        let far: Vec<String> = fleet.book().far().into_iter().map(|far| far.0).collect();
        assert_eq!(far.len(), 2, "after {step}");
        assert!(keys.iter().all(|key| far.contains(key)), "after {step}");
        println!("after {step}: note={:?}", fleet.note());
    }
    let log = fleet.log();
    assert_eq!(log.matches("link up").count(), 2, "{log}");
    assert!(
        !log.contains("route change") && !log.contains("retired"),
        "{log}"
    );
    assert!(!log.contains("unreachable id="), "{log}");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_list_that_parses_again_clears_the_note() {
    let Some((mut fleet, gateway, _)) = one_machine() else {
        return;
    };
    let before = standing(&fleet, &["a"]);
    let (rows, asked) = (fleet.rows(), gateway.world().calls("profiles.list").len());
    let list = [here(), ssh("a", "Mini", "m0")];

    // Unreadable, then readable again as the very same file.
    let file = fleet.list.clone();
    let mode = |mode| std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode));
    mode(0o000).unwrap();
    until("the note is set", || fleet.note().is_some());
    mode(0o600).unwrap();
    until("the note is cleared", || fleet.note().is_none());
    assert_eq!(standing(&fleet, &["a"]), before);

    // Broken, then the same list written anew.
    fleet.write_text("{\"version\": 2, \"connections\": [");
    until("the note is set", || fleet.note().is_some());
    fleet.write_text(&text(&list, "token-9"));
    until("the note is cleared", || fleet.note().is_none());
    fleet.looked(3);
    // With nothing else changed.
    assert_eq!(standing(&fleet, &["a"]), before);
    assert_eq!(fleet.rows(), rows);
    assert_eq!(gateway.world().calls("profiles.list").len(), asked);
    assert_eq!(fleet.log().matches("link up").count(), 1);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_valid_list_with_no_ssh_entries_removes_them() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let first = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let second = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")]);
    fleet.start();
    fleet.connected("Mini");
    fleet.connected("Studio");
    first.asks("default", "s-one", "p-one");
    second.asks("default", "s-two", "p-two");
    until("two cards show", || fleet.cards().len() == 2);

    // A list that parsed, and holds no `ssh` entry at all.
    fleet.write(&[]);
    until("the cards are withdrawn", || fleet.cards().is_empty());
    until("no ssh process is left", || fleet.masters().is_empty());
    assert!(fleet.book().far().is_empty());
    assert_eq!(fleet.note(), None);
    // This machine is still one, with no entry of its own.
    let rows = fleet.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["here"], true);
    println!("machines after a list with no entries: {}", json!(rows));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn launch_with_no_list_dials_nothing() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let _gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    // A binding made for the test: the entry `a` verified before.
    let key = "0123456789abcdef";
    let bound = json!({"version": 1, "bindings": [
        {"entry": "a", "host": "m0", "key": key, "label": "Old Mini"}]});
    let file = fleet.data.join("hermes-machines.json");
    std::fs::write(&file, bound.to_string()).unwrap();

    // No list at all, then one that does not parse.
    for broken in [false, true] {
        if broken {
            fleet.write_text("{\"version\": 2, \"connections\": [{\"id\": \"a\", \"kind\":");
        }
        fleet.start();
        fleet.looked(6);
        let rows = fleet.rows();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["here"], true);
        assert_eq!(rows[1]["id"], key);
        assert_eq!(rows[1]["label"], "Old Mini");
        assert_eq!(rows[1]["state"], "cannot-reach");
        assert!(fleet.note().is_some(), "the list could not be read");
        // Nothing is dialled from what was saved.
        assert!(fleet.masters().is_empty(), "{:?}", fleet.masters());
        assert_eq!(fleet.knocks(0), 0, "nothing reached the sshd");
        assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), bound.to_string());
        println!("machines at launch, broken list={broken}: {}", json!(rows));
        fleet.stop();
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn turning_client_hooks_off_closes_every_link_and_on_brings_them_back() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let first = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let _second = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")]);
    fleet.start();
    let keys = [fleet.connected("Mini"), fleet.connected("Studio")];
    first.asks("default", "s-one", "p-one");
    let id = fleet.card()["id"].as_str().unwrap().to_owned();
    assert_eq!(fleet.masters().len(), 2);

    // Off: what dropping the machines is (`Hooks::configure(false)`).
    fleet.stop();
    assert!(fleet.masters().is_empty(), "{:?}", fleet.masters());
    assert!(fleet.cards().is_empty());
    assert!(
        fleet.rows().is_empty(),
        "no machine is told while it is off"
    );
    let left = crate::ssh_link::unix::tests::lab::entries(&fleet.masters);
    assert!(left.is_empty(), "{left:?}");

    // On: the same machines, under the same keys, and the same request.
    fleet.start();
    assert_eq!([fleet.connected("Mini"), fleet.connected("Studio")], keys);
    assert_eq!(fleet.card()["id"], id.as_str());
    assert_eq!(fleet.masters().len(), 2);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_renamed_entry_changes_the_label_on_a_showing_card_in_place() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    let card = fleet.card();
    assert_eq!(card["machine"], "Mini");
    let (id, times) = (card["id"].as_str().unwrap().to_owned(), fleet.times(&card));
    let sounds = fleet.sounds();
    let before = standing(&fleet, &["a"]);

    fleet.write(&[here(), ssh("a", "Mini in the closet", "m0")]);
    until("the card says the new label", || {
        fleet.card()["machine"] == "Mini in the closet"
    });
    // The same request, changed in place: nothing registered or settled.
    let renamed = fleet.card();
    assert_eq!(renamed["id"], id.as_str());
    assert_eq!(renamed["createdAt"], card["createdAt"]);
    assert_eq!(fleet.times(&renamed), times);
    assert_eq!(fleet.calls(&id), (1, 0));
    assert_eq!(fleet.sounds(), sounds, "no alert");
    assert_eq!(
        standing(&fleet, &["a"])["routes"],
        before["routes"],
        "no reconnect"
    );
    assert_eq!(fleet.connected("Mini in the closet"), key);
    assert!(fleet.row("Mini").is_none());
    // And it is answered as the request it always was.
    (fleet.requests)
        .answer(&id, &pick(&renamed, "one"), crate::hooks::now_ms())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("clarify.respond").is_empty()
    });
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_bindings_survive_a_relaunch_with_the_far_machine_offline() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    let file = fleet.data.join("hermes-machines.json");
    let saved = std::fs::read_to_string(&file).unwrap();
    let bound: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        bound,
        json!({"version": 1, "bindings": [
            {"entry": "a", "host": "m0", "key": key, "label": "Mini"}]}),
        "entry ids, address fields, keys and labels, and nothing else"
    );
    fleet.stop();
    // The far machine goes offline: its gateway, and its sshd.
    gateway.kill();
    let far = fleet.hosts.remove(0);
    let port = far.lab.port;
    drop(far);
    until("the sshd is gone", || {
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
    });

    fleet.start();
    let row = fleet.stands("Mini", "cannot-reach");
    assert_eq!(row["id"], key.as_str(), "its machine, as verified before");
    fleet.looked(5);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
    assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
    println!("machines after a relaunch offline: {}", json!(fleet.rows()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_edited_into_one_that_is_never_dialled_retires_its_machine() {
    // Its address made unusable, and then its kind changed: each time the
    // machine has no entry left that may lead to it.
    let edits = [
        ssh("a", "Mini", "-oProxyCommand=x@m0"),
        json!({"id": "a", "kind": "remote", "label": "Mini", "url": "http://127.0.0.1:9"}),
    ];
    for edit in edits {
        let Some((mut fleet, _gateway, key)) = one_machine() else {
            return;
        };
        fleet.write(&[here(), edit]);
        until("the card is withdrawn", || fleet.cards().is_empty());
        until("no ssh process is left", || fleet.masters().is_empty());
        assert!(fleet.book().far().is_empty());
        assert!(fleet.log().contains(&format!("retired machine={key}")));
        assert_eq!(fleet.stands("Mini", "not-covered")["id"], "a");
    }
}
