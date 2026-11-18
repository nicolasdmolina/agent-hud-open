//! What is not covered and says why, what is never dialled, and what
//! never leaves the list file.
use super::{fleet, here, ssh, text, until, HERE, INSTALL, MARKER};
use crate::clients::hermes::machines::words;
use agent_hud_agents::hermes::{connections::NotCovered, AGENT};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

/// Every file under `folder`, read whole.
fn files(folder: &Path, found: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => files(&path, found),
            Ok(kind) if kind.is_file() => {
                let read = std::fs::read(&path).unwrap_or_default();
                found.push((path.display().to_string(), read));
            }
            _ => {}
        }
    }
}

fn holds(bytes: &[u8], marker: &str) -> bool {
    bytes
        .windows(marker.len())
        .any(|part| part == marker.as_bytes())
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_empty_install_id_is_not_covered_with_that_reason() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", "");
    gateway.asks("default", "s-one", "p-one");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let row = fleet.stands("Mini", "not-covered");
    assert_eq!(row["note"], words::reason("install_id_empty").as_str());
    assert_eq!(row["id"], "a", "no key: it goes by its entry");
    fleet.looked(8);
    // Nothing of it in core: no machine, no card, no reader, no binding.
    assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
    let waiting = fleet.requests.native_waiting_sessions();
    assert!(waiting
        .get(AGENT.id)
        .is_none_or(|sessions| sessions.is_empty()));
    assert!(!fleet.data.join("hermes-machines.json").exists());
    assert!(gateway.world().calls("session.active_list").is_empty());
    println!("machines with an empty install id: {}", json!(fleet.rows()));

    // The same machine is followed once its Hermes says who it is.
    gateway.world().install = Some(INSTALL[0].into());
    let key = fleet.connected("Mini");
    assert_eq!(
        fleet.card()["sessionId"],
        format!("hermes:{key}/default:s-one")
    );
}

/// Ticket 57. A profile's folder that cannot be entered may hold the
/// machine's gateway. Finding none elsewhere is then not word that none
/// runs: the machine goes on connecting, and the log names the profile.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_profile_whose_home_cannot_be_resolved_is_not_word_that_no_gateway_runs() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let shut = fleet.hosts[0].root.join("profiles").join("shut");
    let mode = |mode| fs::set_permissions(&shut, fs::Permissions::from_mode(mode)).unwrap();
    fs::create_dir_all(&shut).unwrap();
    mode(0o000);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    until("the unresolved profile is logged", || {
        fleet.log().contains("profile not resolved profile=shut")
    });
    fleet.looked(3);
    let row = fleet.row("Mini").expect("its status");
    mode(0o700);
    assert_eq!(row["state"], "connecting", "{row}");
    // Every home resolved and no gateway in any: that is the word.
    let row = fleet.stands("Mini", "not-covered");
    assert_eq!(row["note"], words::reason("no_gateway_running").as_str());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn never_verified_and_no_live_connection_is_not_covered() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    gateway.asks("default", "s-one", "p-one");
    // A build whose control socket knows no ticket.
    gateway.world().grants = false;
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);

    // Never verified: not covered, under its entry's id, nothing shown.
    fleet.start();
    let row = fleet.stands("Mini", "not-covered");
    assert_eq!(row["note"], words::reason("no_live_connection").as_str());
    assert_eq!(row["id"], "a");
    fleet.looked(6);
    assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
    assert_eq!(gateway.world().sockets, 0);
    fleet.stop();

    // Verified before, by a binding made for the test: its status stays
    // in the list with its label, as that machine.
    let key = "0123456789abcdef";
    let bound = json!({"version": 1, "bindings": [
        {"entry": "a", "host": "m0", "key": key, "label": "Mini"}]});
    std::fs::write(fleet.data.join("hermes-machines.json"), bound.to_string()).unwrap();
    fleet.start();
    let row = fleet.stands("Mini", "not-covered");
    assert_eq!(row["id"], key);
    assert_eq!(row["label"], "Mini");
    assert_eq!(row["note"], words::reason("no_live_connection").as_str());
    fleet.looked(6);
    assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
    assert_eq!(gateway.world().sockets, 0);
    println!(
        "machines verified before, with no live connection: {}",
        json!(fleet.rows())
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn remote_cloud_and_unknown_kinds_are_not_covered_and_nothing_is_dialed() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let _gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let token = json!({"encoding": "plain", "value": MARKER});
    fleet.write(&[
        here(),
        json!({"id": "r", "kind": "remote", "label": "Remote", "url": "http://127.0.0.1:9",
            "authMode": "token", "token": token, "headers": {"x-extra": MARKER}}),
        json!({"id": "c1", "kind": "cloud", "label": "Cloud", "url": "http://127.0.0.1:9",
            "authMode": "oauth"}),
        json!({"id": "c2", "kind": "cloud", "label": "Cloud, old", "url": "http://127.0.0.1:9",
            "authMode": "token", "token": token}),
        json!({"id": "u", "kind": "satellite", "label": "Unknown", "host": "m0"}),
        ssh("x", "Unusable", "-oProxyCommand=x@m0"),
    ]);
    fleet.start();
    fleet.looked(8);
    let rows = fleet.rows();
    let expected = [
        ("Remote", NotCovered::Remote),
        ("Cloud", NotCovered::Cloud),
        ("Cloud, old", NotCovered::Cloud),
        ("Unknown", NotCovered::UnknownKind),
        ("Unusable", NotCovered::UnusableAddress),
    ];
    assert_eq!(rows.len(), 1 + expected.len(), "{rows:?}");
    assert_eq!(rows[0]["label"], HERE);
    for (row, (label, why)) in rows[1..].iter().zip(expected) {
        assert_eq!(row["label"], label);
        assert_eq!(row["state"], "not-covered", "{row}");
        assert_eq!(row["note"], words::kind(why).as_str(), "{row}");
    }
    // Nothing is dialled: no process, no route, no connection anywhere.
    assert!(fleet.masters().is_empty(), "{:?}", fleet.masters());
    assert_eq!(fleet.knocks(0), 0);
    for entry in ["r", "c1", "c2", "u", "x"] {
        assert!(fleet.book().route_of(entry).is_none(), "{entry}");
    }
    assert!(!fleet.masters.exists() || std::fs::read_dir(&fleet.masters).unwrap().next().is_none());
    assert!(fleet.book().far().is_empty());
    println!("machines of kinds that are not covered: {}", json!(rows));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_marker_in_the_token_and_headers_is_in_no_log_file_or_snapshot() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let token = json!({"encoding": "plain", "value": MARKER});
    let list = [
        here(),
        ssh("a", "Mini", "m0"),
        json!({"id": "r", "kind": "remote", "label": "Remote", "url": "http://127.0.0.1:9",
            "token": token, "headers": {"authorization": MARKER}}),
    ];
    fleet.write_text(&text(&list, MARKER));
    assert!(
        holds(&std::fs::read(&fleet.list).unwrap(), MARKER),
        "the list holds it"
    );
    fleet.start();
    fleet.connected("Mini");
    gateway.asks("default", "s-one", "p-one");
    fleet.card();
    // A list that breaks just after the marker, then one that parses.
    let broken = text(&list, MARKER);
    let cut = broken.find(MARKER).unwrap() + MARKER.len();
    fleet.write_text(&broken[..cut]);
    until("the note is set", || fleet.note().is_some());
    let noted = fleet.snapshot();
    fleet.write_text(&text(&list, MARKER));
    until("the note is cleared", || fleet.note().is_none());
    fleet.looked(3);

    let mut saved = Vec::new();
    files(&fleet.data, &mut saved);
    files(&fleet.masters, &mut saved);
    assert!(saved
        .iter()
        .any(|(path, _)| path.ends_with("hermes-gateway.log")));
    assert!(saved
        .iter()
        .any(|(path, _)| path.ends_with("hermes-machines.json")));
    for (path, bytes) in &saved {
        assert!(!holds(bytes, MARKER), "the marker is in {path}");
    }
    for snapshot in [noted, fleet.snapshot()] {
        assert!(
            !snapshot.to_string().contains(MARKER),
            "the marker is in a snapshot"
        );
    }
    let cards: Value = fleet.cards().into();
    assert!(!cards.to_string().contains(MARKER));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_unknown_host_key_is_not_trusted_and_is_retried_rarely() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let _gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    let known = std::fs::read(fleet.hosts[0].lab.at("known_hosts")).unwrap();
    fleet.write(&[here(), ssh("s", "Stranger", "stranger")]);
    fleet.start();
    let row = fleet.stands("Stranger", "not-trusted");
    assert_eq!(row["note"], words::not_trusted().as_str());
    assert_eq!(row["id"], "s");
    // Retried rarely: once in these seconds, where a host that cannot be
    // reached is tried again within a quarter of a second.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(fleet.knocks(0), 1, "tried once");
    assert_eq!(fleet.logins(0), 0);
    assert_eq!(fleet.stands("Stranger", "not-trusted")["note"], row["note"]);
    // No file of known hosts was written.
    assert_eq!(std::fs::read(&fleet.unknown_hosts).unwrap(), b"");
    assert_eq!(
        std::fs::read(fleet.hosts[0].lab.at("known_hosts")).unwrap(),
        known
    );
    assert!(fleet.book().far().is_empty() && fleet.cards().is_empty());
    println!("machines with an unknown host key: {}", json!(fleet.rows()));
}
