//! The machine in the model: which identity makes a key and which does
//! not, how a gateway gets its anchor, how every session and request id is
//! written, and that far gateways never share a card, a status or an id.
//! Every install id here is made up, and none is ever printed.
use super::super::*;
use super::fake::{identified, names, poll, ready, Hermes};
use super::{far, FAR_HOME, INSTALL, OTHER_INSTALL};
use serde_json::{json, Value};

const S: &str = "20261104_101500_a1b2c3";
/// What `INSTALL` and `OTHER_INSTALL` make at `FAR_HOME`, worked out by
/// hand: `printf '%s\n%s' <id> <home> | shasum -a 256`, first 16.
const KEY: &str = "aaeda597d860920c";
const OTHER_KEY: &str = "1ce13f1418f5a7b9";

fn work() -> Profile {
    Profile::named("work").unwrap()
}

#[test]
fn a_key_is_the_start_of_the_hash_of_the_install_and_its_root_home() {
    let key = machine_key(Some(INSTALL), FAR_HOME).unwrap();
    assert_eq!(key.as_str(), KEY);
    let other = machine_key(Some(OTHER_INSTALL), FAR_HOME).unwrap();
    assert_eq!(other.as_str(), OTHER_KEY);
    // The same pair gives the same key every time.
    for _ in 0..3 {
        assert_eq!(machine_key(Some(INSTALL), FAR_HOME).as_ref(), Ok(&key));
    }
    // One install in two root homes is two machines.
    let copy = machine_key(Some(INSTALL), "/Users/USER/hermes-copy").unwrap();
    assert_eq!(copy.as_str(), "adcabf2b930ca672");
    assert_ne!(copy, key);
    // A key is no piece of the id it came from.
    assert!(!INSTALL.contains(key.as_str()) && !key.as_str().contains(&INSTALL[..8]));
}

#[test]
fn an_install_id_hermes_would_not_write_makes_no_key_and_says_why() {
    let upper = INSTALL.to_uppercase();
    let (short, long) = (&INSTALL[..31], format!("{INSTALL}0"));
    let not_hex = INSTALL.replace('f', "g");
    let padded = format!("{}\n", &INSTALL[..31]);
    let cases: [(&str, Option<&str>, NoKey); 9] = [
        ("missing", None, NoKey::Missing),
        ("empty", Some(""), NoKey::Empty),
        ("31 characters", Some(short), NoKey::Length),
        ("33 characters", Some(&long), NoKey::Length),
        ("upper case", Some(&upper), NoKey::UpperCase),
        (
            "one upper case",
            Some("0123456789abcdef0123456789abcdeF"),
            NoKey::UpperCase,
        ),
        ("not hex", Some(&not_hex), NoKey::NotHex),
        ("a line end in it", Some(&padded), NoKey::NotHex),
        (
            "a UUID with dashes",
            Some("01234567-89ab-cdef-0123-456789abcdef"),
            NoKey::Length,
        ),
    ];
    let mut reasons = std::collections::BTreeSet::new();
    for (case, install_id, why) in cases {
        // The case is named, never the id.
        assert_eq!(machine_key(install_id, FAR_HOME), Err(why), "{case}");
        reasons.insert(why.reason_code());
    }
    // Missing, empty, a wrong length, upper case and not hex each say so.
    assert_eq!(reasons.len(), 5);
    // A home that was not made canonical is no identity either.
    for home in ["", "~/.hermes", ".hermes", "Users/USER/.hermes"] {
        assert_eq!(
            machine_key(Some(INSTALL), home),
            Err(NoKey::Home),
            "{home:?}"
        );
    }
    assert_eq!(NoKey::Home.reason_code(), "root_home_not_canonical");
}

#[test]
fn gateways_on_one_route_must_report_one_install() {
    let (a, b) = (Some(INSTALL), Some(OTHER_INSTALL));
    assert!(identities_agree([a]));
    assert!(identities_agree([a, a, a]));
    assert!(!identities_agree([a, b]));
    assert!(!identities_agree([a, a, b]));
    assert!(!identities_agree([b, a, a]));
    // One that names none disagrees with one that names an install.
    assert!(!identities_agree([a, None]));
    assert!(!identities_agree([Some(""), a]));
    // Agreeing on nothing is agreeing, and still makes no key.
    assert!(identities_agree([None, Some(""), None]));
    assert_eq!(machine_key(None, FAR_HOME), Err(NoKey::Missing));
    // A route with no gateway has none that disagree.
    assert!(identities_agree(Vec::<Option<&str>>::new()));
}

#[test]
fn machine_profile_and_id_make_the_hud_session_id() {
    let (mini, studio) = (far(INSTALL), far(OTHER_INSTALL));
    // A profile of this machine named as a far machine's key is.
    let keyed = Profile::named(KEY).unwrap();
    let table = [
        (Machine::This, Profile::Default, format!("hermes:{S}")),
        (Machine::This, work(), format!("hermes:work:{S}")),
        (Machine::This, keyed, format!("hermes:{KEY}:{S}")),
        (
            mini.clone(),
            Profile::Default,
            format!("hermes:{KEY}/default:{S}"),
        ),
        (mini.clone(), work(), format!("hermes:{KEY}/work:{S}")),
        (
            studio.clone(),
            Profile::Default,
            format!("hermes:{OTHER_KEY}/default:{S}"),
        ),
        (studio, work(), format!("hermes:{OTHER_KEY}/work:{S}")),
    ];
    let mut ids = std::collections::BTreeSet::new();
    for (machine, profile, expected) in &table {
        assert_eq!(hud_session_id(machine, profile, S).as_ref(), Some(expected));
        // A session of any gateway on the machine is that session.
        for anchor in [Profile::Default, work()] {
            let gateway = GatewayId::new(machine.clone(), anchor);
            let session = SessionRef::new(gateway, profile.clone(), S).unwrap();
            assert_eq!(session.hud_id(), *expected);
        }
        ids.insert(expected.clone());
    }
    // The same profile and id on this machine and on two far ones are
    // three sessions.
    assert_eq!(ids.len(), table.len());
    // The reader and the hooks write this machine's forms: `transcript_tests`
    // and `hooks::tests` hold their values, unchanged.
    let default = Profile::named("default").unwrap();
    assert_eq!(
        hud_session_id(&mini, &default, S).as_ref(),
        Some(&table[3].2)
    );
}

fn homes(answer: &Value) -> Vec<&str> {
    let served = answer["served_profiles"].as_array().unwrap();
    served
        .iter()
        .map(|entry| entry["home"].as_str().unwrap())
        .collect()
}

#[test]
fn the_anchor_is_the_default_profile_or_the_first_home() {
    let root = "/Users/USER/.hermes";
    let (alpha, reports) = (
        "/Users/USER/.hermes/profiles/alpha",
        "/Users/USER/.hermes/profiles/reports",
    );
    let named = |name: &str| Profile::named(name);
    // The default profile, wherever the gateway lists it.
    for listed in [
        [root, alpha, reports],
        [alpha, root, reports],
        [reports, alpha, root],
    ] {
        assert_eq!(anchor(root, listed), Some(Profile::Default), "{listed:?}");
    }
    // Without it, the first by home, in whatever order they come.
    for listed in [[alpha, reports], [reports, alpha]] {
        assert_eq!(anchor(root, listed), named("alpha"), "{listed:?}");
    }
    // One gateway for each profile, as the Mini runs them.
    assert_eq!(anchor(root, [reports]), named("reports"));
    assert_eq!(anchor(root, [root]), Some(Profile::Default));
    assert_eq!(anchor(root, []), None);
    // A home that is no profile of this root names nothing, and does not
    // stand in the way of one that is.
    let elsewhere = [
        "/Users/USER/other/profiles/aaa",
        "/Users/USER/.hermes/profiles",
        "/Users/USER/.hermes/profiles/alpha/",
        "/Users/USER/.hermes/profiles/alpha/profiles/aaa",
        "/Users/USER/.hermes/profiles/Alpha",
        "/Users/USER/.hermes/profiles/default",
        "/Users/USER/.hermes/",
    ];
    assert_eq!(anchor(root, elsewhere), None);
    let mixed = elsewhere.into_iter().chain([reports]);
    assert_eq!(anchor(root, mixed), named("reports"));
    // A root that is not written as a canonical path is has no profiles.
    for root in ["", "/", "/Users/USER/.hermes/", "Users/USER/.hermes"] {
        assert_eq!(anchor(root, [root]), None, "{root:?}");
    }
}

#[test]
fn a_restart_keeps_the_anchor_the_gateway_id_and_its_request_ids() {
    let mut requests = Vec::new();
    let runs = [
        ("instance-1", ["verify", "ops"]),
        ("instance-2", ["ops", "verify"]),
    ];
    let mut gateway: Option<Gateway> = None;
    for (instance, served) in runs {
        let answer = ready(instance, &served);
        let id = GatewayId::new(far(INSTALL), anchor("/synthetic", homes(&answer)).unwrap());
        assert_eq!(id.anchor(), &Profile::named("ops").unwrap(), "{instance}");
        assert_eq!(id.to_string(), format!("{KEY}/ops"));
        // The follower keeps its model across the restart; the id it would
        // be given again is the one it has.
        let model = gateway.get_or_insert_with(|| Gateway::new(id.clone()));
        assert_eq!(model.id(), &id);
        let home = "/synthetic/profiles/verify";
        model.apply(
            Input::Identified {
                answer: &answer,
                home,
                root: "/synthetic",
            },
            0,
        );
        let mut hermes = Hermes::default();
        let asked = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
            "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
        hermes.publish(
            &Profile::named("verify").unwrap(),
            S,
            "clarify.request",
            1,
            asked,
        );
        poll(model, &mut hermes, 0);
        requests.push(model.waiting().next().expect("a card").id());
    }
    assert_eq!(requests[0], format!("hermes:{KEY}/ops:verify:{S}:p1"));
    assert_eq!(requests[0], requests[1]);
}

#[test]
fn a_gateway_is_written_by_its_machine_and_its_anchor() {
    let mini = far(INSTALL);
    let written = |machine: &Machine, anchor: Profile| {
        let id = GatewayId::new(machine.clone(), anchor);
        let session = SessionRef::new(id.clone(), work(), S).unwrap();
        (id.to_string(), session.request_id("p1"))
    };
    let table = [
        (written(&Machine::This, Profile::Default), "local"),
        (written(&Machine::This, work()), "local/work"),
        (written(&mini, Profile::Default), "aaeda597d860920c/default"),
        (written(&mini, work()), "aaeda597d860920c/work"),
        (
            written(&far(OTHER_INSTALL), work()),
            "1ce13f1418f5a7b9/work",
        ),
    ];
    for ((gateway, request), expected) in table {
        assert_eq!(gateway, expected);
        // Ticket 12's form: agent, gateway, profile, session, what was asked.
        assert_eq!(request, format!("hermes:{expected}:work:{S}:p1"));
    }
}

/// A gateway that is ready, and its Hermes with one question in `verify`.
fn asking(id: GatewayId) -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(id);
    identified(&mut gateway, "instance-1");
    let mut hermes = Hermes::default();
    let asked = json!({"kind": "clarify", "prompt_id": "same", "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
    hermes.publish(
        &Profile::named("verify").unwrap(),
        S,
        "clarify.request",
        1,
        asked,
    );
    (gateway, hermes)
}

/// Two gateways with the same profile, session id and prompt id.
fn never_mix(one: GatewayId, other: GatewayId, one_machine: bool) {
    let (mut a, mut ha) = asking(one);
    let (mut b, mut hb) = asking(other);
    for (gateway, hermes) in [(&mut a, &mut ha), (&mut b, &mut hb)] {
        let effects = poll(gateway, hermes, 0);
        assert!(matches!(&effects[..], [Effect::Open(_)]), "{effects:?}");
    }
    let verify = Profile::named("verify").unwrap();
    let (at_a, at_b) = (
        a.session(&verify, S).unwrap(),
        b.session(&verify, S).unwrap(),
    );
    let (id_a, id_b) = (at_a.request_id("same"), at_b.request_id("same"));
    // Request ids.
    assert_ne!(id_a, id_b);
    assert_eq!(
        a.waiting().map(Pending::id).collect::<Vec<_>>(),
        std::slice::from_ref(&id_a)
    );
    assert_eq!(
        b.waiting().map(Pending::id).collect::<Vec<_>>(),
        std::slice::from_ref(&id_b)
    );
    assert_eq!((a.pending(&id_b), b.pending(&id_a)), (None, None));
    // Cards: each view carries its own id, and its own machine's session.
    let (card_a, card_b) = (a.pending(&id_a).unwrap(), b.pending(&id_b).unwrap());
    assert_eq!(card_a.view(0, None)["id"], id_a.as_str());
    assert_eq!(card_b.view(0, None)["id"], id_b.as_str());
    assert_eq!(card_a.view(0, None)["sessionId"], at_a.hud_id().as_str());
    assert_eq!(card_b.view(0, None)["sessionId"], at_b.hud_id().as_str());
    // Statuses.
    assert_eq!(a.statuses(), [(at_a.clone(), Status::Waiting)]);
    assert_eq!(b.statuses(), [(at_b.clone(), Status::Waiting)]);
    assert_eq!((a.status(&at_b), b.status(&at_a)), (None, None));
    // The session is one session only where the machine is one machine.
    assert_eq!(at_a.hud_id() == at_b.hud_id(), one_machine);
    // Only this machine's session has the id the hooks and the reader write.
    for at in [&at_a, &at_b] {
        let here = *at.gateway.machine() == Machine::This;
        assert_eq!(at.hud_id() == format!("hermes:verify:{S}"), here);
    }
    // A poll of one, given to the other, says nothing there.
    let mut theirs = b.poll().unwrap();
    super::fake::ask(&mut theirs, &mut hb);
    assert_eq!(a.apply(Input::Polled(theirs), 1), []);
    // Neither does an answer to the other's request.
    let reply = json!({"result": {"status": "resolved"}});
    let answered = Input::Responded {
        request: &id_b,
        reply: &reply,
    };
    assert_eq!(a.apply(answered, 2), []);
    assert_eq!(names(&a), ["same"]);
    // One answered, one restarted: the other keeps its card and status.
    let answered = Input::Responded {
        request: &id_a,
        reply: &reply,
    };
    let effects = a.apply(answered, 3);
    assert!(matches!(&effects[0], Effect::Close { id, session, .. }
        if *id == id_a && *session == at_a));
    assert_eq!(names(&b), ["same"]);
    let effects = identified(&mut b, "instance-2");
    assert!(matches!(&effects[0], Effect::Close { id, session, .. }
        if *id == id_b && *session == at_b));
    assert!(names(&a).is_empty() && names(&b).is_empty());
}

#[test]
fn two_gateways_on_one_far_machine_never_mix() {
    let mini = far(INSTALL);
    let one = GatewayId::new(mini.clone(), Profile::Default);
    never_mix(
        one,
        GatewayId::new(mini, Profile::named("reports").unwrap()),
        true,
    );
}

#[test]
fn one_gateway_on_each_of_two_machines_never_mix() {
    let one = GatewayId::new(far(INSTALL), Profile::Default);
    never_mix(
        one.clone(),
        GatewayId::new(far(OTHER_INSTALL), Profile::Default),
        false,
    );
    // A far machine's gateway and this machine's.
    never_mix(GatewayId::new(Machine::This, Profile::Default), one, false);
}

#[test]
fn the_install_id_is_in_no_id_and_no_debug_output() {
    let (mut gateway, mut hermes) = asking(GatewayId::new(far(INSTALL), work()));
    let effects = poll(&mut gateway, &mut hermes, 0);
    let pending = gateway.waiting().next().unwrap().clone();
    let begun = gateway.poll().unwrap();
    let written = [
        format!("{:?}", far(INSTALL)),
        format!("{:?}", machine_key(Some(INSTALL), FAR_HOME)),
        format!("{:?} {}", gateway.id(), gateway.id()),
        format!("{:?} {}", pending.session, pending.session.hud_id()),
        format!("{pending:?} {} {}", pending.id(), pending.view(0, None)),
        format!("{gateway:?} {begun:?} {effects:?}"),
        format!("{:?}", gateway.statuses()),
        format!("{:?}", machine_key(Some(&INSTALL.to_uppercase()), FAR_HOME)),
        format!("{:?}", machine_key(Some(&INSTALL[..31]), FAR_HOME)),
    ];
    for (n, text) in written.iter().enumerate() {
        // Named by its place: a failure must not print what it found.
        let lower = text.to_lowercase();
        assert!(
            !lower.contains(INSTALL) && !lower.contains(&INSTALL[..31]),
            "entry {n}"
        );
    }
    assert!(written[2].contains(KEY) && written[3].contains(KEY));
}
