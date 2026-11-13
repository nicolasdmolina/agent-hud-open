//! What the first review of the machine identity found at its edges
//! (`evidence/22/verifier-files/boundary-probes.rs`), under the names it
//! gave them, and what guards the same edges elsewhere. Each failed
//! before its fix: one folder written two ways gave two machines, a root
//! lying in a folder called `profiles` was taken for a named profile, and a
//! hook on this machine could name a far machine's session.
use super::super::*;
use super::fake::{identified, names, poll, Hermes};
use super::{far, INSTALL};
use serde_json::{json, Value};

const S: &str = "20261104_101500_a1b2c3";
const NOW: i64 = 1_790_000_000_000;
/// `INSTALL` at `/srv/hermes`, worked out by hand as `machines.rs` says.
const SRV_KEY: &str = "19dc3a759af7f225";

// ---- 1. Equivalent root paths made different machine keys ----

#[test]
fn equivalent_homes_are_canonicalized_or_rejected() {
    let base = machine_key(Some(INSTALL), "/srv/hermes").unwrap();
    // The key of a canonical home is what it was.
    assert_eq!(base.as_str(), SRV_KEY);
    let cases = [
        ("trailing slash", "/srv/hermes/"),
        ("repeated separator", "/srv//hermes"),
        ("dot segment", "/srv/./hermes"),
        ("parent segment", "/srv/other/../hermes"),
        ("double leading slash", "//srv/hermes"),
        ("embedded NUL", "/srv/hermes\0"),
        ("NUL inside", "/srv/\0hermes"),
        ("ending dot", "/srv/hermes/."),
        ("ending parent", "/srv/hermes/x/.."),
        ("the root alone", "/"),
        ("nothing", ""),
        ("relative", "srv/hermes"),
        ("a home sign", "~/.hermes"),
    ];
    for (case, home) in cases {
        // Refused, not tidied: nothing here can resolve a link, so the far
        // side hands over the one canonical form or gets no key.
        assert_eq!(machine_key(Some(INSTALL), home), Err(NoKey::Home), "{case}");
    }
    // Names that only look like something to resolve are plain folders.
    for home in [
        "/srv/.hermes",
        "/srv/..hermes",
        "/srv/her.mes/a b",
        "/srv/…",
    ] {
        assert!(machine_key(Some(INSTALL), home).is_ok(), "{home}");
    }
}

// ---- 2. A root under a folder named `profiles` collided with a named gateway ----

#[test]
fn a_default_root_under_a_profiles_folder_still_anchors_default() {
    // HERMES_HOME is arbitrary. These are already canonical far paths.
    let root = "/srv/profiles/work";
    let named = "/srv/profiles/work/profiles/work";
    let root_anchor = anchor(root, [root]).unwrap();
    let named_anchor = anchor(root, [named]).unwrap();
    assert_eq!(root_anchor, Profile::Default);
    assert_eq!(Some(&named_anchor), Profile::named("work").as_ref());
    for machine in [Machine::This, far(INSTALL)] {
        let root_gateway = GatewayId::new(machine.clone(), root_anchor.clone());
        let named_gateway = GatewayId::new(machine, named_anchor.clone());
        // Two gateways, two reader names and two request namespaces.
        assert_ne!(root_gateway, named_gateway);
        assert_ne!(root_gateway.to_string(), named_gateway.to_string());
    }
    for homes in [[root, named], [named, root]] {
        assert_eq!(anchor(root, homes), Some(Profile::Default), "{homes:?}");
    }
    // Only the root says which it is: the same home is a named profile of
    // the root two folders up, and no profile of any other.
    assert_eq!(Profile::in_root("/srv", root), Profile::named("work"));
    assert_eq!(Profile::in_root("/srv/profiles", root), None);
    assert_eq!(Profile::in_root("/srv/profiles/work/profiles", named), None);
}

// ---- 3. The local hook boundary could emit a far session id ----

fn hook(payload: &Value) -> Option<String> {
    let event = crate::parse_event("hermes", "--completion-hook", payload, NOW).unwrap();
    event.map(|event| event.session_id)
}

fn began(session: &str, profile: Option<&str>) -> Value {
    let mut payload = json!({
        "hook_event_name": "pre_llm_call", "session_id": session,
        "cwd": "/synthetic/project", "extra": {"turn_id": "synthetic-turn"}
    });
    if let Some(profile) = profile {
        payload["profile"] = profile.into();
    }
    payload
}

#[test]
fn a_local_hook_cannot_name_a_far_session() {
    let key = machine_key(Some(INSTALL), "/srv/hermes").unwrap();
    let mini = Machine::Far(key.clone());
    let remote = hud_session_id(&mini, &Profile::Default, S).unwrap();
    assert_eq!(remote, format!("hermes:{SRV_KEY}/default:{S}"));
    let tail = remote.strip_prefix("hermes:").unwrap();
    let profile = format!("{SRV_KEY}/default");
    // No profile can be made with that name, so no id can be built on it.
    assert_eq!(Profile::named(&profile), None);
    assert_eq!(Profile::folder(&profile), None);
    assert_eq!(
        hud_session_id(&Machine::This, &Profile::Default, tail),
        None
    );
    // Through the real entry point, as Hermes runs a hook.
    assert_eq!(hook(&began(S, Some(&profile))), None, "the profile");
    assert_eq!(hook(&began(tail, None)), None, "the session id");
    let by_home = {
        let mut payload = began(S, None);
        let home = format!("/Users/USER/.hermes/profiles/{profile}");
        crate::hooks::add_environment("hermes", &mut payload, &|name| {
            (name == "HERMES_HOME").then(|| home.clone())
        });
        payload
    };
    assert_ne!(hook(&by_home), Some(remote.clone()), "the hook's home");
    // An approval names its session by a key (`extra.session_key`).
    let approval = json!({"hook_event_name": "pre_approval_request", "session_id": "",
        "extra": {"session_key": tail, "surface": "cli"}});
    assert_eq!(hook(&approval), None, "the session key");
    // The same hooks with what Hermes writes are what they were.
    assert_eq!(hook(&began(S, None)), Some(format!("hermes:{S}")));
    assert_eq!(
        hook(&began(S, Some("work"))),
        Some(format!("hermes:work:{S}"))
    );
    assert_eq!(
        hook(&began(S, Some("default"))),
        Some(format!("hermes:{S}"))
    );
}

#[test]
fn a_profile_is_named_only_as_hermes_names_one() {
    let long = "a".repeat(64);
    for name in [
        "work",
        "hud-spike",
        "a",
        "0",
        "9_lives",
        "a-b_c",
        SRV_KEY,
        long.as_str(),
    ] {
        let profile = Profile::named(name).expect(name);
        assert_eq!(profile.name(), name);
        assert_eq!(Profile::folder(name), Some(profile));
    }
    for name in ["", "default"] {
        assert_eq!(Profile::named(name), Some(Profile::Default), "{name:?}");
    }
    let too_long = "a".repeat(65);
    let malformed = [
        "a/b",
        "/",
        "a:b",
        "..",
        ".",
        ".hidden",
        "-a",
        "_a",
        "Work",
        "wörk",
        "a b",
        " work",
        "work ",
        " default ",
        " ",
        "a\0",
        "a\n",
        "19dc3a759af7f225/default",
        "local/work",
        too_long.as_str(),
    ];
    for name in malformed {
        assert_eq!(Profile::named(name), None, "{name:?}");
        assert_eq!(Profile::folder(name), None, "{name:?}");
        let home = format!("/srv/hermes/profiles/{name}");
        assert_eq!(Profile::in_root("/srv/hermes", &home), None, "{name:?}");
    }
    // A folder is named exactly: `default` is no named profile's folder.
    assert_eq!(Profile::folder("default"), None);
    assert_eq!(Profile::folder(" work"), None);
}

#[test]
fn no_id_of_this_machine_is_a_far_machines() {
    let mini = far(INSTALL);
    let profiles = [
        Profile::Default,
        Profile::named("work").unwrap(),
        Profile::named(SRV_KEY).unwrap(),
    ];
    // Before the first colon after `hermes:`, a far id holds a slash and
    // a local one never does.
    let marked = |id: &str| {
        let tail = id.strip_prefix("hermes:").unwrap();
        tail.split(':').next().unwrap().contains('/')
    };
    let ids = [
        S,
        "local-f200386683510182",
        "agent:main:telegram:dm:12345",
        "agent:main:signal:group:Ab/Cd==",
        "a",
        "a:b/c",
        "a/b",
        "/",
        "/a",
        "a/b:c",
        "19dc3a759af7f225/default:x",
        "aaeda597d860920c/work:x",
        "",
    ];
    for profile in &profiles {
        for id in ids {
            let local = hud_session_id(&Machine::This, profile, id);
            let distant = hud_session_id(&mini, profile, id);
            // The same ids are refused wherever they come from.
            assert_eq!(local.is_some(), distant.is_some(), "{profile:?} {id:?}");
            let refused = id.is_empty() || id.split(':').next().unwrap().contains('/');
            assert_eq!(local.is_none(), refused, "{profile:?} {id:?}");
            if let (Some(local), Some(distant)) = (local, distant) {
                assert!(!marked(&local) && marked(&distant), "{local} {distant}");
            }
            // A session is named by the same rule, so its id is always written.
            let gateway = GatewayId::new(Machine::This, Profile::Default);
            let session = SessionRef::new(gateway, profile.clone(), id);
            assert_eq!(session.is_none(), refused, "{profile:?} {id:?}");
        }
    }
    // What Hermes writes stays byte for byte what it was.
    let this = |profile: &Profile, id: &str| hud_session_id(&Machine::This, profile, id).unwrap();
    assert_eq!(this(&profiles[0], S), format!("hermes:{S}"));
    assert_eq!(this(&profiles[1], S), format!("hermes:work:{S}"));
    assert_eq!(
        this(&profiles[0], "agent:main:signal:group:Ab/Cd=="),
        "hermes:agent:main:signal:group:Ab/Cd=="
    );
}

#[test]
fn a_listed_session_with_no_session_id_is_not_followed() {
    let mut gateway = Gateway::new(GatewayId::new(Machine::This, Profile::Default));
    identified(&mut gateway, "instance-1");
    let mut hermes = Hermes::default();
    let asked = |prompt: &str| {
        json!({"kind": "clarify", "prompt_id": prompt, "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false})
    };
    // This machine's gateway lists a session named as a far one is.
    let forged = format!("{SRV_KEY}/default:{S}");
    hermes.publish(
        &Profile::Default,
        &forged,
        "clarify.request",
        1,
        asked("forged"),
    );
    hermes.publish(&Profile::Default, S, "clarify.request", 1, asked("real"));
    poll(&mut gateway, &mut hermes, 0);
    assert_eq!(names(&gateway), ["real"]);
    let told: Vec<String> = gateway
        .statuses()
        .iter()
        .map(|(at, _)| at.hud_id())
        .collect();
    assert_eq!(told, [format!("hermes:{S}")]);
    assert_eq!(gateway.session(&Profile::Default, &forged), None);
}

#[test]
fn a_served_home_in_a_folder_that_is_no_profiles_is_not_served() {
    let mut answer = super::fake::ready("instance-1", &["default", "verify"]);
    let served = answer["served_profiles"].as_array_mut().unwrap();
    let home = format!("/synthetic/profiles/{SRV_KEY}:default");
    served.push(json!({"profile_id": home, "home": home}));
    let Identified::Ready(instance) = identify(&answer, "/synthetic") else {
        panic!("ready");
    };
    let profiles: Vec<&str> = instance.served.iter().map(|s| s.profile.name()).collect();
    assert_eq!(profiles, ["default", "verify"]);
}
