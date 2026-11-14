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
fn an_instance_is_named_by_the_anchor_of_the_homes_it_serves() {
    for served in [
        vec!["default", "verify"],
        vec!["verify", "default"],
        vec!["verify", "ops"],
        vec!["ops", "verify"],
        vec!["ops"],
    ] {
        let answer = ready("instance-1", &served);
        let home = answer["served_profiles"][0]["home"].as_str().unwrap();
        let Identified::Ready(instance) = identify(&answer, home, ROOT) else {
            panic!("ready: {served:?}");
        };
        let homes = answer["served_profiles"].as_array().unwrap();
        let homes = homes.iter().map(|entry| entry["home"].as_str().unwrap());
        assert_eq!(instance.anchor(), anchor(ROOT, homes), "{served:?}");
        assert!(instance.anchor().is_some());
    }
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
    let (home, root) = (ROOT, ROOT);
    gateway.apply(
        Input::Identified {
            answer: &answer,
            home,
            root,
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
