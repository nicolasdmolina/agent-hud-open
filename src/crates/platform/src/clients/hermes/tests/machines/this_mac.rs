//! Ticket 69. This Mac's row in the machines list says what its own
//! followers know, as a far machine's does: connected only while a
//! gateway here is followed. Each test runs with the list's entry for
//! this machine and with no entry for it, the two ways the row is made.
use super::{super::fake::Fake, fleet, here, until, Fleet, HERE};
use crate::clients::hermes::machines::words;
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

/// The two lists: one with this machine's entry, one without. Each with
/// the label this machine's row goes by then.
fn lists() -> [(Vec<Value>, String); 2] {
    [
        (vec![here()], HERE.to_owned()),
        (vec![], words::this_machine()),
    ]
}

/// Waits until this machine's row stands as `state` and says `note`, and
/// checks that it is this machine's. A socket that closes is said first
/// as a gateway that cannot be reached, then as what the next dial finds.
fn stands(fleet: &mut Fleet, label: &str, state: &str, note: Option<String>) {
    let mut found = None;
    until(&format!("{label} is {state}, {note:?}"), || {
        let row = fleet.row(label).filter(|row| row["state"] == state);
        found = row.filter(|row| row["note"].as_str() == note.as_deref());
        found.is_some()
    });
    assert_eq!(found.unwrap()["here"], true);
}

/// What the row says through a while of looks: the same thing at each.
fn stays(fleet: &mut Fleet, label: &str, state: &str) {
    for _ in 0..20 {
        fleet.looked(3);
        let row = fleet.row(label).expect("this machine is listed");
        assert_eq!(row["state"], state, "{row}");
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn this_mac_is_connected_only_while_its_gateway_is_followed() {
    for (list, label) in lists() {
        let Some(mut fleet) = fleet(1) else {
            return;
        };
        fleet.write(&list);
        fleet.start();
        // A Hermes home with no gateway in it.
        let none = Some(words::here_no_gateway());
        stands(&mut fleet, &label, "cannot-reach", none.clone());
        stays(&mut fleet, &label, "cannot-reach");

        let mut gateway = Fake::start(&fleet.local, "instance-local");
        stands(&mut fleet, &label, "connected", None);
        println!("this machine ({label}), followed: {:?}", fleet.rows());

        // The gateway stops, as `hermes gateway stop` does.
        gateway.stop();
        stands(&mut fleet, &label, "cannot-reach", none);
        println!("this machine ({label}): {:?}", fleet.rows());
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_mac_with_no_hermes_home_is_not_a_failure() {
    for (list, label) in lists() {
        let Some(mut fleet) = fleet(1) else {
            return;
        };
        fs::remove_dir(&fleet.local).unwrap();
        fleet.write(&list);
        fleet.start();
        stands(
            &mut fleet,
            &label,
            "not-covered",
            Some(words::here_no_hermes()),
        );
        stays(&mut fleet, &label, "not-covered");
        assert_eq!(fleet.note(), None, "no warning on the whole list");

        // Hermes is set up afterwards: its home is there, no gateway yet.
        crate::os::private_directory(&fleet.local).unwrap();
        let none = Some(words::here_no_gateway());
        stands(&mut fleet, &label, "cannot-reach", none);
        println!("this machine ({label}): {:?}", fleet.rows());
    }
}

/// A profile's folder that cannot be entered, opened again when dropped
/// so the lab can remove it.
struct Shut(PathBuf);

impl Drop for Shut {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

/// Ticket 57's rule, on this machine: where no gateway is followed and a
/// profile's home could not be asked, that is not word that none runs.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_closed_profile_here_keeps_this_mac_connecting() {
    for (list, label) in lists() {
        let Some(mut fleet) = fleet(1) else {
            return;
        };
        let shut = Shut(fleet.local.join("profiles").join("shut"));
        fs::create_dir_all(&shut.0).unwrap();
        fs::set_permissions(&shut.0, fs::Permissions::from_mode(0o000)).unwrap();
        fleet.write(&list);
        fleet.start();
        // Many looks at the machine, each one finding the folder closed.
        stays(&mut fleet, &label, "connecting");

        drop(shut);
        let none = Some(words::here_no_gateway());
        stands(&mut fleet, &label, "cannot-reach", none);
        println!("this machine ({label}): {:?}", fleet.rows());
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_old_hermes_here_is_not_covered() {
    for (list, label) in lists() {
        let Some(mut fleet) = fleet(1) else {
            return;
        };
        // A build whose control socket knows no ticket.
        let old = Fake::start(&fleet.local, "instance-old");
        old.world().grants = false;
        fleet.write(&list);
        fleet.start();
        let note = Some(words::here_no_live_connection());
        stands(&mut fleet, &label, "not-covered", note);
        stays(&mut fleet, &label, "not-covered");
        println!("this machine ({label}): {:?}", fleet.rows());
    }
}
