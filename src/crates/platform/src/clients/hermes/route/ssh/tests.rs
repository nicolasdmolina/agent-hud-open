//! The parts of the route over SSH that need no link: which origin a
//! forward may lead to, which machine a route leads to, and what it says
//! to whoever follows it. The route itself is tested through a throwaway
//! `sshd` (`tests/far.rs`).
use super::{
    asked_profile,
    asking::silence,
    far_port,
    identity::{Identity, NoMachine},
    notice::{Notice, Noticed},
};
use crate::{
    clients::hermes::{control::Silent, far},
    ssh_link::{Failure as LinkFailure, Lost, Refused},
};
use agent_hud_agents::hermes::gateway::far::{Quiet, Unchecked};
use agent_hud_agents::hermes::gateway::{
    identify, machine_key, Identified, Instance, NoKey, Profile,
};
use serde_json::json;

const INSTALL: &str = "0123456789abcdef0123456789abcdef";
const OTHER: &str = "fedcba9876543210fedcba9876543210";
const ROOT: &str = "/Users/far/.hermes";

/// An instance as the model accepts it, with this origin.
fn accepted(origin: &str) -> Option<Instance> {
    let named = format!("{ROOT}/profiles/reports");
    let answer = json!({"ok": true, "result": {
        "runtime_protocol": 1, "state": "ready", "instance_id": "instance-1",
        "authority_epoch": 1, "api_origin": origin,
        "capabilities": ["session-authority-v1"],
        "served_profiles": [
            {"profile_id": ROOT, "home": ROOT},
            {"profile_id": named, "home": named},
        ],
    }});
    match identify(&answer, &named) {
        Identified::Ready(instance) => Some(instance),
        _ => None,
    }
}

#[test]
fn a_forward_leads_only_to_the_port_of_an_origin_on_the_far_127_0_0_1() {
    let port = |origin: &str| accepted(origin).as_ref().and_then(far_port);
    assert_eq!(port("http://127.0.0.1:8642"), Some(8642));
    assert_eq!(port("http://127.0.0.1:8642/"), Some(8642));
    // The model takes these for the machine itself; no forward leads there.
    for origin in [
        "http://127.0.0.2:8642",
        "http://[::1]:8642",
        "http://127.1:8642",
    ] {
        assert!(accepted(origin).is_some(), "{origin}");
        assert_eq!(port(origin), None, "{origin}");
    }
    // And these never become an instance, so nothing is asked of the route.
    for origin in [
        "http://10.1.2.3:8642",
        "https://127.0.0.1:8642",
        "http://localhost:8642",
        "http://127.0.0.1",
        "http://user@127.0.0.1:8642",
    ] {
        assert!(accepted(origin).is_none(), "{origin}");
    }
    // The port is the instance's own, never one read from elsewhere.
    let mut moved = accepted("http://127.0.0.1:8642").unwrap();
    moved.port = 9000;
    assert_eq!(far_port(&moved), None);
    moved.origin = "http://127.0.0.1:0".into();
    moved.port = 0;
    assert_eq!(far_port(&moved), None);
}

#[test]
fn a_call_on_a_socket_names_the_profile_whose_home_was_asked() {
    let instance = accepted("http://127.0.0.1:8642").unwrap();
    assert_eq!(asked_profile(&instance), Profile::named("reports").unwrap());
}

fn reported(ids: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    let own =
        |(instance, id): &(&str, Option<&str>)| ((*instance).to_owned(), id.map(str::to_owned));
    ids.iter().map(own).collect()
}

#[test]
fn every_gateway_must_report_the_one_install_that_makes_a_key() {
    let identity = Identity::default();
    let settle = |ids: &[(&str, Option<&str>)]| identity.settle(ROOT, &reported(ids), 1);
    assert_eq!(settle(&[]), Err(NoMachine::Unverified));
    assert_eq!(
        settle(&[("a", None)]),
        Err(NoMachine::NoKey(NoKey::Missing))
    );
    assert_eq!(
        settle(&[("a", Some(""))]),
        Err(NoMachine::NoKey(NoKey::Empty))
    );
    let two = [("a", Some(INSTALL)), ("b", Some(OTHER))];
    assert_eq!(settle(&two), Err(NoMachine::Disagree));
    assert_eq!(
        settle(&[("a", Some(INSTALL)), ("b", None)]),
        Err(NoMachine::Disagree)
    );
    // Nothing was verified by any of that.
    assert!(!identity.admitted(1, "a"));
    assert_eq!(
        identity.admit(1, "a", Some(INSTALL)),
        Err(NoMachine::Unverified)
    );
    assert!(identity.rooted_at("/anywhere"));

    let key = settle(&[("a", Some(INSTALL)), ("b", Some(INSTALL))]).unwrap();
    assert_eq!(key, machine_key(Some(INSTALL), ROOT).unwrap());
    assert!(identity.admitted(1, "a") && identity.admitted(1, "b"));
    assert!(identity.rooted_at(ROOT) && !identity.rooted_at("/Users/far/other"));
}

#[test]
fn a_later_gateway_is_held_against_the_machine_on_every_link() {
    let identity = Identity::default();
    identity
        .settle(ROOT, &reported(&[("a", Some(INSTALL))]), 1)
        .unwrap();
    // One that turns up later says who it is before it is admitted.
    assert!(!identity.admitted(1, "c"));
    assert_eq!(identity.admit(1, "c", Some(OTHER)), Err(NoMachine::Another));
    assert_eq!(
        identity.admit(1, "c", None),
        Err(NoMachine::NoKey(NoKey::Missing))
    );
    assert_eq!(
        identity.admit(1, "c", Some("")),
        Err(NoMachine::NoKey(NoKey::Empty))
    );
    assert!(!identity.admitted(1, "c"));
    assert_eq!(identity.admit(1, "c", Some(INSTALL)), Ok(()));
    assert!(identity.admitted(1, "c") && identity.admitted(1, "a"));

    // Another link: nobody is admitted until asked again on it.
    assert!(!identity.admitted(2, "a") && !identity.admitted(2, "c"));
    assert_eq!(identity.admit(2, "a", Some(INSTALL)), Ok(()));
    assert!(identity.admitted(2, "a") && !identity.admitted(2, "c"));
    assert!(!identity.admitted(1, "a"), "the old link's are forgotten");

    identity.forget();
    assert!(!identity.admitted(2, "a"));
    assert_eq!(
        identity.admit(2, "a", Some(INSTALL)),
        Err(NoMachine::Unverified)
    );
}

#[test]
fn the_first_reason_a_machine_is_uncovered_for_is_the_one_kept() {
    let notice = Notice::default();
    assert_eq!(notice.wait(&|| true), Noticed::Over);
    notice.doubt();
    assert_eq!(notice.wait(&|| false), Noticed::Doubt);
    notice.doubt();
    notice.uncover("install_id_changed");
    notice.uncover("root_home_changed");
    let first = Noticed::Uncovered("install_id_changed".into());
    assert_eq!(notice.wait(&|| false), first);
    assert_eq!(notice.wait(&|| false), first, "said until it is cleared");
    notice.clear();
    assert_eq!(notice.wait(&|| true), Noticed::Over);
}

/// Only the relay's own word says that nobody listens. A follower takes
/// its gateway for gone on that word alone, and gives up what it claims.
#[test]
fn nothing_but_the_relays_own_word_says_that_nobody_listens() {
    assert_eq!(silence(far::Silent::Relay(Quiet::Absent)), Silent::Absent);
    let lost = [
        Lost::NotTrusted,
        Lost::KeyChanged,
        Lost::SignInRefused,
        Lost::CannotReach,
        Lost::TimedOut,
        Lost::Ended,
    ];
    let mut others: Vec<far::Silent> = lost
        .into_iter()
        .map(|lost| far::Silent::Link(LinkFailure::Lost(lost)))
        .collect();
    others.extend([
        far::Silent::Link(LinkFailure::Refused(Refused::Closed)),
        far::Silent::Link(LinkFailure::Refused(Refused::Unavailable)),
        far::Silent::CutOff,
        far::Silent::Busy,
        far::Silent::Relay(Quiet::Empty),
        far::Silent::Relay(Quiet::TimedOut),
        far::Silent::Relay(Quiet::TooLong),
        far::Silent::Relay(Quiet::Garbled),
        far::Silent::Relay(Quiet::Failed),
        far::Silent::Relay(Quiet::Unchecked(Unchecked::ListenerNotYours)),
        far::Silent::Relay(Quiet::Unchecked(Unchecked::NoPython)),
    ]);
    for silent in others {
        assert_ne!(silence(silent), Silent::Absent, "{silent:?}");
    }
    // A lost link is something that was not heard, which a look counts
    // as a claim it missed.
    let ended = far::Silent::Link(LinkFailure::Lost(Lost::Ended));
    assert_eq!(silence(ended), Silent::TimedOut);
    let unsafe_home = far::Silent::Relay(Quiet::Unchecked(Unchecked::HomeOpenToOthers));
    assert_eq!(silence(unsafe_home), Silent::Unsafe("home-open-to-others"));
}
