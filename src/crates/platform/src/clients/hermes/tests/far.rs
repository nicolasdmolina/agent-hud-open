//! The route over SSH, through ticket 28's throwaway `sshd` on loopback
//! (`ssh_link/unix/tests/lab.rs`). "Far" is this same machine, signed in
//! as the user who runs the tests, with a home folder of the lab's own:
//! no test looks at the user's real Hermes home, and none reads the
//! user's `ssh` files. The far gateways are the pretend ones of the other
//! tests (`fake.rs`), in that home, so the same followers, polls and
//! answers run here as there, over a relay and a forwarded port.
//!
//! The tests start processes, so they are marked `#[ignore]` and run by
//! `the_lab_tests_pass` in a process of their own. To run them here and
//! see what each prints:
//! `cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far:: -- --ignored --nocapture`.
pub(super) use super::super::Standing;
use super::{
    super::{machine::Inner, FarEntry, FarObserver, Pace},
    fake::Fake,
    quick,
};
use crate::ssh_link::{
    unix::tests::lab::{self, far as command, Lab},
    End, Link,
};
use agent_hud_agents::hermes::gateway::MachineKey;
use agent_hud_core::requests::Requests;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "far/cards.rs"]
mod cards;
#[path = "far/identity.rs"]
mod identity;
#[path = "far/links.rs"]
pub(super) mod links;
#[path = "far/review30.rs"]
mod review30;
#[path = "far/rules.rs"]
mod rules;

#[test]
fn the_lab_tests_pass() {
    lab::in_a_process_of_their_own("clients::hermes::tests_unix::far::");
}

/// What the lab's Hermes reports as its install.
pub(super) const INSTALL: &str = "0123456789abcdef0123456789abcdef";
pub(super) const OTHER_INSTALL: &str = "fedcba9876543210fedcba9876543210";
/// What the far machine is called on its cards.
pub(super) const LABEL: &str = "Lab Mini";

/// A far machine: the lab's `sshd`, a far home folder of the lab's own
/// with a Hermes root in it, the app's data folder, and core's requests.
pub(super) struct FarLab {
    /// The Hermes root on the far side, `<lab>/far/.hermes`, as the far
    /// side resolves it.
    pub root: PathBuf,
    pub data: PathBuf,
    pub requests: Requests,
    pub observer: Option<FarObserver>,
    pub lab: Lab,
}

/// The pace of the far tests: the tests' own, with a look at the machine
/// by the clock and waits long enough for a relay on a busy machine.
pub(super) fn pace() -> Pace {
    Pace {
        discover: Some(Duration::from_millis(500)),
        answer: Duration::from_secs(6),
        ..quick()
    }
}

/// Waits for `met`, with a limit for a machine busy with other builds: a
/// relay is a process on the far side each time.
pub(super) fn until(what: &str, mut met: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(90);
    while !met() {
        assert!(Instant::now() < limit, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The far lab, or None where this machine cannot run an `sshd`.
pub(super) fn far_lab() -> Option<FarLab> {
    let lab = Lab::start_with(&[("HOME", "{lab}/far")])?;
    let dir = std::fs::canonicalize(lab.at("")).expect("the lab's folder");
    let (root, data) = (dir.join("far").join(".hermes"), dir.join("data"));
    crate::os::private_directory(&dir.join("far")).unwrap();
    crate::os::private_directory(&root).unwrap();
    crate::os::private_directory(&data).unwrap();
    // Before anything asks about Hermes: a login's home folder is the
    // lab's, so nothing below can look at the user's own.
    let link = Link::open(&lab.folder(), &lab.target(), &lab.settings()).expect("the link");
    let ran = link.run(&command("printf %s \"$HOME\"", &[])).expect("ran");
    assert_eq!(ran.end, End::Ended(0));
    let home = std::fs::canonicalize(String::from_utf8_lossy(&ran.stdout).as_ref());
    assert_eq!(home.ok(), Some(dir.join("far")), "the far home folder");
    link.close();
    Some(FarLab {
        root,
        data,
        requests: Requests::default(),
        observer: None,
        lab,
    })
}

impl FarLab {
    /// A gateway in the far root that reports the lab's install.
    pub(super) fn gateway(&self, instance: &str) -> Fake {
        let gateway = Fake::start(&self.root, instance);
        gateway.world().install = Some(INSTALL.into());
        gateway
    }

    /// Starts following the far machine, as the app will for an entry of
    /// the connection list.
    pub(super) fn follow(&mut self) {
        self.follow_at(pace());
    }

    pub(super) fn follow_at(&mut self, pace: Pace) {
        let entry = FarEntry {
            target: self.lab.target(),
            label: LABEL.to_owned(),
            folder: self.lab.folder(),
            settings: self.lab.settings(),
        };
        let (requests, wake) = (self.requests.clone(), Arc::new(|| {}));
        let observer = FarObserver::start_at(entry, &self.data, requests, wake, pace);
        self.observer = Some(observer.expect("the far observer"));
    }

    pub(super) fn observer(&self) -> &FarObserver {
        self.observer.as_ref().expect("following")
    }

    pub(super) fn inner(&self) -> &Arc<Inner> {
        self.observer().inner()
    }

    pub(super) fn standing(&self) -> Standing {
        self.observer().standing()
    }

    /// Waits until the machine is verified and followed: its key.
    pub(super) fn followed(&self) -> MachineKey {
        let mut key = None;
        until("the far machine is followed", || {
            if let Standing::Connected(found) = self.standing() {
                key = Some(found);
            }
            key.is_some()
        });
        key.unwrap()
    }

    pub(super) fn cards(&self) -> Vec<Value> {
        self.requests.views(crate::hooks::now_ms())
    }

    /// Waits until exactly one card shows and returns it.
    pub(super) fn card(&self) -> Value {
        until("one card shows", || self.cards().len() == 1);
        self.cards().remove(0)
    }

    pub(super) fn log(&self) -> String {
        std::fs::read_to_string(self.data.join("hermes-gateway.log")).unwrap_or_default()
    }

    /// What the lab's `sshd` wrote: every login is a line of it.
    pub(super) fn logins(&self) -> usize {
        let said = std::fs::read_to_string(self.lab.at("sshd.log")).unwrap_or_default();
        said.matches("Accepted publickey").count()
    }

    pub(super) fn answer(&self, id: &str, decision: &Value) {
        let taken = self.requests.answer(id, decision, crate::hooks::now_ms());
        taken.expect("core takes the decision");
    }

    pub(super) fn delivery(&self, id: &str) -> Value {
        let found = |row: &Value| row["id"] == id;
        let rows = || self.requests.deliveries(crate::hooks::now_ms());
        until("the answer has a receipt", || rows().iter().any(found));
        rows().into_iter().find(found).unwrap()
    }
}

/// A test that fails says what the connection logged.
impl Drop for FarLab {
    fn drop(&mut self) {
        // The observer first, so its master is closed while `sshd` runs.
        self.observer.take();
        if std::thread::panicking() {
            eprintln!("the connection's log:\n{}", self.log());
        }
    }
}
