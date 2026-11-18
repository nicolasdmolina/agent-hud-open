//! Ticket 62. A far machine with a profile whose home could not be
//! resolved: what is still followed, and how often the log says so.
use super::{fleet, here, ssh, until, Host, INSTALL};
use crate::clients::hermes::machines::words;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

/// A profile's folder that cannot be entered. It can be again when this
/// is dropped, so that the lab can remove its folder whatever a test did.
struct Shut(PathBuf);

impl Shut {
    fn new(host: &Host, name: &str) -> Self {
        let shut = Self(host.root.join("profiles").join(name));
        fs::create_dir_all(&shut.0).unwrap();
        shut.mode(0o000);
        shut
    }

    fn mode(&self, mode: u32) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(mode));
    }
}

impl Drop for Shut {
    fn drop(&mut self) {
        self.mode(0o700);
    }
}

/// How many times the log names `profile` as not resolved, on the lines
/// of the host `host`.
fn said(log: &str, host: &str, profile: &str) -> usize {
    let line = format!("profile not resolved profile={profile}");
    (log.lines())
        .filter(|said| said.contains(host) && said.ends_with(&line))
        .count()
}

/// The verifier's probe for ticket 57. An unresolved profile keeps a
/// machine connecting only where no gateway was found at all. A gateway
/// that answers is followed without that profile, and an old one without
/// the live connection is still the reason the machine is not covered.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_gateway_that_answers_is_followed_with_a_profile_unresolved() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    let _shut = [0, 1].map(|host| Shut::new(&fleet.hosts[host], "shut"));
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    gateway.asks("default", "s-one", "p-one");
    // A build whose control socket knows no ticket.
    let old = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    old.world().grants = false;
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")]);
    fleet.start();
    let key = fleet.connected("Mini");
    let studio = fleet.stands("Studio", "not-covered");
    assert_eq!(studio["note"], words::reason("no_live_connection").as_str());
    fleet.looked(3);
    assert_eq!(fleet.row("Mini").expect("its status")["state"], "connected");
    assert_eq!(
        fleet.card()["sessionId"],
        format!("hermes:{key}/default:s-one")
    );
    assert_eq!(said(&fleet.log(), "m0", "shut"), 1, "{}", fleet.log());
}

/// A profile that stays unresolved is named in the log once, not at every
/// look: the log is one small file that every machine shares. It is named
/// again only after it was resolved in between.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_unresolved_profile_is_logged_once_until_it_changes() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    let count = |fleet: &super::Fleet, profile: &str| said(&fleet.log(), "m0", profile);
    let shut = Shut::new(&fleet.hosts[0], "shut");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    until("shut is named", || count(&fleet, "shut") > 0);
    // Each new name is word of a later look, at which the old ones stood.
    let _barred = Shut::new(&fleet.hosts[0], "barred");
    until("barred is named", || count(&fleet, "barred") > 0);
    let _locked = Shut::new(&fleet.hosts[0], "locked");
    until("locked is named", || count(&fleet, "locked") > 0);
    let names = ["shut", "barred", "locked"];
    assert_eq!(names.map(|name| count(&fleet, name)), [1, 1, 1]);
    assert_eq!(
        fleet.row("Mini").expect("its status")["state"],
        "connecting"
    );

    // Resolved at one look and not at a later one: it changed, and is said.
    shut.mode(0o700);
    let _closed = Shut::new(&fleet.hosts[0], "closed");
    until("closed is named", || count(&fleet, "closed") > 0);
    shut.mode(0o000);
    until("shut is named again", || count(&fleet, "shut") > 1);
    let _sealed = Shut::new(&fleet.hosts[0], "sealed");
    until("sealed is named", || count(&fleet, "sealed") > 0);
    assert_eq!(names.map(|name| count(&fleet, name)), [2, 1, 1]);
}
