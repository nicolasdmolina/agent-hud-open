//! A gateway's profiles as they change while it is followed (ticket 26):
//! a profile that left it closes its cards, a profile two instances claim
//! gets no new card, and a gateway that came back as another instance
//! leaves nothing behind.
use super::fake::{identified, names, poll, ready, Hermes};
use super::*;
use serde_json::json;
use std::collections::BTreeSet;

const ROOT: &str = "/synthetic";

fn verify() -> Profile {
    Profile::named("verify").unwrap()
}

fn set(profiles: &[Profile]) -> BTreeSet<Profile> {
    profiles.iter().cloned().collect()
}

/// A question with the prompt id `name`, waiting in `session`.
fn asks(hermes: &mut Hermes, profile: &Profile, session: &str, name: &str) {
    let asked = json!({"kind": "clarify", "prompt_id": name, "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
    hermes.publish(profile, session, "clarify.request", 1, asked);
}

/// A gateway that is ready, serving the default profile and `verify`,
/// with a card in each.
fn two_cards() -> (Gateway, Hermes) {
    let mut gateway = local();
    identified(&mut gateway, "instance-1");
    let mut hermes = Hermes::default();
    asks(&mut hermes, &Profile::Default, "s-default", "in-default");
    asks(&mut hermes, &verify(), "s-verify", "in-verify");
    poll(&mut gateway, &mut hermes, 0);
    assert_eq!(names(&gateway), ["in-default", "in-verify"]);
    (gateway, hermes)
}

fn closed(effects: &[Effect]) -> Vec<(String, Closed)> {
    let close = |effect: &Effect| match effect {
        Effect::Close { id, why, .. } => Some((id.clone(), why.clone())),
        _ => None,
    };
    effects.iter().filter_map(close).collect()
}

#[test]
fn a_gateway_is_anchored_by_the_homes_its_answer_serves() {
    for served in [
        vec!["default", "verify"],
        vec!["verify", "default"],
        vec!["verify", "ops"],
        vec!["ops", "verify"],
        vec!["ops"],
    ] {
        let answer = ready("instance-1", &served);
        let homes = served_homes(&answer);
        assert_eq!(homes.len(), served.len(), "{served:?}");
        let first = if served.contains(&"default") {
            Profile::Default
        } else {
            Profile::named("ops").unwrap()
        };
        assert_eq!(anchor(ROOT, homes), Some(first), "{served:?}");
        // The same homes inside the envelope a control socket answers in.
        let whole = json!({"ok": true, "protocol": 1, "id": 1, "result": answer});
        assert_eq!(served_homes(&whole), served_homes(&answer));
    }
    assert!(served_homes(&json!({"ok": false, "error": "no"})).is_empty());
}

/// Two gateways of one machine that would go by one anchor: the second
/// is told apart by its instance, in a form no other gateway's id has.
#[test]
fn a_gateway_told_apart_from_one_of_the_same_anchor_has_a_name_of_its_own() {
    let alpha = Profile::named("alpha").unwrap();
    let kept = GatewayId::new(Machine::This, alpha.clone());
    let apart = GatewayId::apart(Machine::This, alpha.clone(), "zzzzzzzz-new-alpha");
    assert_eq!(kept.to_string(), "local/alpha");
    let written = apart.to_string();
    let (name, hash) = written.split_once('~').unwrap();
    assert_eq!(name, "local/alpha");
    let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    assert!(hash.len() == 8 && hash.chars().all(hex), "{written}");
    assert_ne!(apart, kept);
    assert_eq!(apart.anchor(), &alpha);
    // The same for as long as the gateway runs as that instance, and
    // another's for another.
    let again = GatewayId::apart(Machine::This, alpha.clone(), "zzzzzzzz-new-alpha");
    assert_eq!(again, apart);
    let other = GatewayId::apart(Machine::This, alpha.clone(), "yyyyyyyy-another");
    assert_ne!(other.to_string(), written);
    // No profile is named so, so no gateway is anchored at such a name.
    assert_eq!(
        Profile::named(written.strip_prefix("local/").unwrap()),
        None
    );
    assert_eq!(Profile::named(&format!("alpha~{hash}")), None);
    // The default anchor and a far machine are told apart the same way.
    let own = GatewayId::apart(Machine::This, Profile::Default, "i");
    assert!(own.to_string().starts_with("local/default~"), "{own}");
    let far = GatewayId::apart(far(INSTALL), alpha.clone(), "i").to_string();
    assert!(far.starts_with("aaeda597d860920c/alpha~"), "{far}");
    // Its sessions are the profile's own; its requests carry its name.
    let session = SessionRef::new(apart.clone(), alpha, "s1").unwrap();
    assert_eq!(session.hud_id(), "hermes:alpha:s1");
    let request = session.request_id("p1");
    assert_eq!(request, format!("hermes:{written}:alpha:s1:p1"));
}

#[test]
fn a_profile_that_left_the_gateway_closes_its_cards_and_is_listed_no_more() {
    let (mut gateway, mut hermes) = two_cards();
    assert_eq!(gateway.holds(), &set(&[Profile::Default, verify()]));
    let effects = gateway.apply(Input::Retired(&set(&[verify()])), 10);
    let id = "hermes:local:verify:s-verify:in-verify".to_owned();
    assert_eq!(closed(&effects), [(id, Closed::Retired)]);
    assert_eq!(names(&gateway), ["in-default"]);
    assert_eq!(gateway.holds(), &set(&[Profile::Default]));
    let statuses = gateway.statuses();
    assert!(statuses
        .iter()
        .all(|(at, _)| at.profile == Profile::Default));
    // No poll lists it again: the profile is another gateway's now.
    poll(&mut gateway, &mut hermes, 20);
    let listed = |step: &Step| matches!(step, Step::List(profile) if *profile == verify());
    assert!(!hermes.asked.iter().any(listed), "{:?}", hermes.asked);
    assert_eq!(names(&gateway), ["in-default"]);
    // Retiring it again, or a profile it never had, changes nothing.
    let ops = Profile::named("ops").unwrap();
    assert!(gateway
        .apply(Input::Retired(&set(&[verify(), ops])), 30)
        .is_empty());
}

#[test]
fn a_poll_under_way_when_a_profile_is_retired_is_dropped() {
    let (mut gateway, mut hermes) = two_cards();
    let mut under_way = gateway.poll().unwrap();
    super::fake::ask(&mut under_way, &mut hermes);
    gateway.apply(Input::Retired(&set(&[verify()])), 10);
    // It listed the retired profile and found its request still waiting.
    let effects = gateway.apply(Input::Polled(under_way), 20);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(names(&gateway), ["in-default"]);
}

#[test]
fn a_profile_is_held_while_the_gateway_is_out_of_reach_and_can_still_be_retired() {
    let (mut gateway, _) = two_cards();
    gateway.apply(Input::Lost(Failure::Closed(1006)), 10);
    assert_eq!(gateway.holds(), &set(&[Profile::Default, verify()]));
    assert_eq!(
        names(&gateway),
        ["in-default", "in-verify"],
        "the cards stay"
    );
    let effects = gateway.apply(Input::Retired(&set(&[verify()])), 20);
    assert_eq!(closed(&effects).len(), 1);
    assert_eq!(gateway.holds(), &set(&[Profile::Default]));
    assert_eq!(names(&gateway), ["in-default"]);
}

#[test]
fn the_gateway_saying_it_serves_a_profile_again_takes_it_back() {
    let (mut gateway, mut hermes) = two_cards();
    gateway.apply(Input::Retired(&set(&[verify()])), 10);
    let answer = ready("instance-1", &["default", "verify"]);
    let home = ROOT;
    gateway.apply(
        Input::Identified {
            answer: &answer,
            home,
        },
        20,
    );
    assert_eq!(gateway.holds(), &set(&[Profile::Default, verify()]));
    let effects = poll(&mut gateway, &mut hermes, 30);
    assert!(matches!(&effects[..], [Effect::Open(_)]), "{effects:?}");
    assert_eq!(names(&gateway), ["in-default", "in-verify"]);
}

#[test]
fn no_card_opens_for_a_profile_two_instances_claim_and_the_ones_that_show_are_kept() {
    let (mut gateway, mut hermes) = two_cards();
    let contested = set(&[verify()]);
    assert!(gateway.apply(Input::Withheld(&contested), 10).is_empty());
    assert_eq!(gateway.withheld(), &contested);
    // The card that shows stays; a new request there opens none, while
    // one in the other profile opens as ever.
    asks(&mut hermes, &verify(), "s-verify", "second-in-verify");
    asks(
        &mut hermes,
        &Profile::Default,
        "s-default",
        "second-in-default",
    );
    let effects = poll(&mut gateway, &mut hermes, 20);
    assert!(matches!(&effects[..], [Effect::Open(_)]), "{effects:?}");
    assert_eq!(
        names(&gateway),
        ["in-default", "second-in-default", "in-verify"]
    );
    // The profile is still polled, and a card that shows closes as
    // Hermes says.
    let listed = |step: &Step| matches!(step, Step::List(profile) if *profile == verify());
    assert!(hermes.asked.iter().any(listed));
    let settled = json!({"prompt_id": "in-verify", "execution_generation": 1});
    hermes.publish(&verify(), "s-verify", "clarify.settled", 1, settled);
    let effects = poll(&mut gateway, &mut hermes, 30);
    assert_eq!(closed(&effects).len(), 1, "{effects:?}");
    assert_eq!(names(&gateway), ["in-default", "second-in-default"]);
    // Being told the same again asks for nothing.
    assert!(gateway.apply(Input::Withheld(&contested), 40).is_empty());
    // One instance claims it now: a poll is asked for at once, and opens
    // what waits.
    let effects = gateway.apply(Input::Withheld(&BTreeSet::new()), 50);
    assert_eq!(effects, [Effect::Poll]);
    let effects = poll(&mut gateway, &mut hermes, 60);
    assert!(matches!(&effects[..], [Effect::Open(_)]), "{effects:?}");
    assert_eq!(
        names(&gateway),
        ["in-default", "second-in-default", "second-in-verify"]
    );
}

#[test]
fn a_gateway_that_is_gone_closes_every_card_as_a_restart() {
    let (mut gateway, _) = two_cards();
    gateway.apply(Input::Lost(Failure::Closed(1012)), 10);
    let effects = gateway.apply(Input::Restarted, 20);
    let whys: Vec<Closed> = closed(&effects).into_iter().map(|(_, why)| why).collect();
    assert_eq!(whys, [Closed::GatewayRestarted, Closed::GatewayRestarted]);
    assert_eq!(effects.len(), 2, "nothing is said of the connection");
    assert!(names(&gateway).is_empty());
    assert!(gateway.statuses().is_empty());
    // The instance it followed is still the one it names: the follower of
    // the new instance is another model.
    assert_eq!(gateway.instance(), Some("instance-1"));
}

/// A home that lies at `…/profiles/<name>` is the profile `<name>` to
/// Hermes, wherever its root is (the second review of ticket 26 ran it):
/// its calls are routed by that name, and its sessions are named by it,
/// as the hooks and the `state.db` reader name them.
#[test]
fn a_home_in_a_profiles_folder_is_named_and_routed_by_its_path() {
    let home = "/srv/profiles/work";
    let answer = json!({"runtime_protocol": 1, "state": "ready", "instance_id": "instance-1",
        "authority_epoch": 1, "api_origin": "http://127.0.0.1:54321",
        "capabilities": [SESSION_AUTHORITY],
        "served_profiles": [{"profile_id": home, "home": home}]});
    let mut gateway = local();
    gateway.apply(
        Input::Identified {
            answer: &answer,
            home,
        },
        0,
    );
    let work = Profile::named("work").unwrap();
    assert_eq!(gateway.holds(), &set(std::slice::from_ref(&work)));
    let mut hermes = Hermes::default();
    asks(&mut hermes, &work, "s1", "p1");
    poll(&mut gateway, &mut hermes, 0);
    assert!(hermes.asked.contains(&Step::List(work.clone())));
    for step in &hermes.asked {
        assert_eq!(step.command(Scope::Host).params["profile"], "work");
    }
    // Whoever knows the machine's root home names the gateway by it, and
    // nothing else: where the home is the root, the gateway is `local`.
    assert_eq!(anchor(home, served_homes(&answer)), Some(Profile::Default));
    let ids: Vec<String> = gateway.waiting().map(Pending::id).collect();
    assert_eq!(ids, ["hermes:local:work:s1:p1"]);
    let sessions: Vec<String> = gateway.waiting().map(|p| p.session.hud_id()).collect();
    assert_eq!(sessions, ["hermes:work:s1"]);
    let told = gateway.live_states().sessions;
    let told: Vec<String> = told.into_iter().map(|(id, _)| id).collect();
    assert_eq!(told, ["hermes:work:s1"]);
    // An answer goes back by the same name.
    let pending = gateway.pending("hermes:local:work:s1:p1").unwrap();
    let decision = json!({"kind": "answer", "answers": {"p1": {"selected": ["a"]}}});
    let Ok(Some(Reply::Call(command))) = pending.reply(&decision, Scope::Host) else {
        panic!("an answer to send");
    };
    assert_eq!(command.params["profile"], "work");
    let at = gateway.session(&work, "s1").unwrap();
    assert_eq!(resume(&at, Scope::Host).params["profile"], "work");
}
