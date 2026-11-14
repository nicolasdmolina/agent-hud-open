//! Several gateways on one machine (ticket 26): every Hermes home is
//! asked who is there, the answers are put together by instance, and each
//! gateway found has a follower of its own, named by its anchor profile.
//! A look that reaches only some homes changes nothing; a follower ends
//! only when its gateway is gone, and never for being out of reach.
use super::super::{Pace, Shared};
use super::{
    fake::{home_of, Fake},
    lab, quick, until, Lab,
};
use serde_json::Value;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

/// The tests' pace with a look at the machine every few hundredths of a
/// second, as the app takes one every ten seconds.
pub fn looking() -> Pace {
    Pace {
        discover: Some(Duration::from_millis(40)),
        ..quick()
    }
}

impl Lab {
    /// Starts following every gateway of the home.
    pub fn follow_all(&mut self) {
        self.follow_at(looking());
    }
    /// The followers there are, by the name each tells core under.
    pub fn readers(&self) -> Vec<String> {
        let observer = self.observer.as_ref().unwrap();
        let mut readers: Vec<String> = observer
            .followers()
            .iter()
            .map(|follower| follower.reader.clone())
            .collect();
        readers.sort();
        readers
    }
    pub fn follower(&self, reader: &str) -> Arc<Shared> {
        let followers = self.observer.as_ref().unwrap().followers();
        let named = |follower: &Arc<Shared>| follower.reader == reader;
        followers.into_iter().find(named).unwrap()
    }
    /// The id of every card that shows, sorted.
    pub fn shown(&self) -> Vec<String> {
        let id = |card: &Value| card["id"].as_str().unwrap().to_owned();
        let mut ids: Vec<String> = self.cards().iter().map(id).collect();
        ids.sort();
        ids
    }
    /// Waits for `looks` more looks at the machine to have begun and the
    /// one before the last of them to be over.
    pub fn looks(&self, looks: usize) {
        let before = self.route.asked().homes;
        until("the machine is looked at again", || {
            self.route.asked().homes > before + looks
        });
    }
    /// The lines of the log that hold `what`, without their time.
    pub fn said(&self, what: &str) -> Vec<String> {
        let log = self.log();
        let lines = log.lines().filter(|line| line.contains(what));
        lines
            .filter_map(|line| line.split_once(' '))
            .map(|(_, said)| said.to_owned())
            .collect()
    }
}

/// Every profile the gateway was asked to list, without repeats.
fn listed(gateway: &Fake) -> Vec<String> {
    let calls = gateway.world().calls("session.active_list");
    let profile =
        |(_, frame): &(usize, Value)| frame["params"]["profile"].as_str().unwrap().to_owned();
    let mut profiles: Vec<String> = calls.iter().map(profile).collect();
    profiles.sort();
    profiles.dedup();
    profiles
}

#[test]
fn one_gateway_serving_two_profiles_has_one_follower_and_cards_from_both() {
    // The MacBook's layout: one gateway, one control socket in the root
    // home, every profile served by it.
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    crate::os::private_directory(&home_of(&lab.home, "hud-spike")).unwrap();
    gateway.asks("default", "s-default", "p1");
    gateway.asks("hud-spike", "s-spike", "p2");
    lab.follow_all();
    let both = [
        "hermes:local:default:s-default:p1",
        "hermes:local:hud-spike:s-spike:p2",
    ];
    until("a card from each profile shows", || lab.shown() == both);
    lab.looks(3);
    assert_eq!(lab.readers(), ["local"]);
    assert_eq!(lab.shown(), both);
    assert!(lab.said("gateway ").is_empty(), "{}", lab.log());
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    assert_eq!(listed(&gateway), ["default", "hud-spike"]);
}

#[test]
fn one_gateway_for_each_of_two_profiles_has_two_followers_and_no_call_names_the_others_profile() {
    // The Mini's layout: each profile has a gateway of its own, with its
    // control socket in its own home.
    let mut lab = lab();
    let default = Fake::serving(&lab.home, "aaaaaaaa-default", &["default"]);
    let reports = Fake::serving(&lab.home, "bbbbbbbb-reports", &["reports"]);
    default.asks("default", "s-default", "p1");
    reports.asks("reports", "s-reports", "p2");
    lab.follow_all();
    let both = [
        "hermes:local/reports:reports:s-reports:p2",
        "hermes:local:default:s-default:p1",
    ];
    until("a card from each gateway shows", || lab.shown() == both);
    assert_eq!(lab.readers(), ["local", "local/reports"]);
    // Each session is told to core by the follower of its own gateway.
    let states = lab.requests.native_states();
    let read = |reader: &str| {
        let reading = states.of("hermes", reader).unwrap();
        reading.sessions.keys().cloned().collect::<Vec<_>>()
    };
    assert_eq!(read("local"), ["hermes:s-default"]);
    assert_eq!(read("local/reports"), ["hermes:reports:s-reports"]);
    lab.looks(3);
    // No call ever named the other gateway's profile.
    assert_eq!(listed(&default), ["default"]);
    assert_eq!(listed(&reports), ["reports"]);
    for gateway in [&default, &reports] {
        let world = gateway.world();
        assert!(world.mismatched.is_empty(), "{:?}", world.mismatched);
    }
    assert!(!lab.log().contains("profile_mismatch"), "{}", lab.log());
    assert_eq!(
        lab.said("gateway found"),
        ["gateway found id=local/reports instance=bbbbbbbb profiles=1"]
    );
    // The lines of the second gateway begin with its id.
    let ready = "local/reports connection ready instance=bbbbbbbb profiles=1 draining=false";
    assert_eq!(lab.said("local/reports connection"), [ready]);
}

/// The followers and cards of three gateways: one for the default profile,
/// one for `alpha` and `beta` with a socket in each, one for `gamma`.
fn three(backwards: bool) -> (Vec<String>, Vec<String>) {
    let mut lab = lab();
    lab.route.backwards.store(backwards, Ordering::Release);
    let default = Fake::serving(&lab.home, "instance-default", &["default"]);
    let two = Fake::serving(&lab.home, "instance-two", &["beta", "alpha"]);
    let gamma = Fake::serving(&lab.home, "instance-gamma", &["gamma"]);
    default.asks("default", "s1", "p1");
    two.asks("alpha", "s2", "p2");
    two.asks("beta", "s3", "p3");
    gamma.asks("gamma", "s4", "p4");
    lab.follow_all();
    until("four cards show", || lab.shown().len() == 4);
    lab.looks(3);
    (lab.readers(), lab.shown())
}

#[test]
fn the_same_gateways_are_found_with_the_same_ids_whichever_home_is_asked_first() {
    let (readers, cards) = three(false);
    assert_eq!(readers, ["local", "local/alpha", "local/gamma"]);
    let expected = [
        "hermes:local/alpha:alpha:s2:p2",
        "hermes:local/alpha:beta:s3:p3",
        "hermes:local/gamma:gamma:s4:p4",
        "hermes:local:default:s1:p1",
    ];
    assert_eq!(cards, expected);
    assert_eq!(three(true), (readers, cards));
}

#[test]
fn a_discovery_that_times_out_on_one_home_changes_no_follower_and_no_card() {
    let mut lab = lab();
    let default = Fake::serving(&lab.home, "instance-default", &["default"]);
    let reports = Fake::serving(&lab.home, "instance-reports", &["reports"]);
    default.asks("default", "s-default", "p1");
    reports.asks("reports", "s-reports", "p2");
    lab.follow_all();
    until("a card from each gateway shows", || lab.shown().len() == 2);
    let (followers, cards) = (lab.observer.as_ref().unwrap().followers(), lab.cards());
    let log = lab.log().lines().count();
    for slow in [vec!["reports"], vec!["default"], vec!["default", "reports"]] {
        *lab.route.slow.lock().unwrap() = slow.iter().map(|name| (*name).to_owned()).collect();
        lab.looks(4);
        let now = lab.observer.as_ref().unwrap().followers();
        assert_eq!(now.len(), 2, "{slow:?}");
        for (was, is) in followers.iter().zip(&now) {
            assert!(Arc::ptr_eq(was, is), "{slow:?}");
        }
        assert_eq!(lab.cards(), cards, "{slow:?}");
    }
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    // Nothing was said either: no gateway found or gone, no card touched.
    assert_eq!(lab.log().lines().count(), log, "{}", lab.log());
    // The homes answer again, and everything is as it was.
    lab.route.slow.lock().unwrap().clear();
    lab.looks(3);
    assert_eq!(lab.cards(), cards);
    assert_eq!(lab.readers(), ["local", "local/reports"]);
}

#[test]
fn a_profile_created_while_the_hud_runs_is_covered_by_the_next_discovery() {
    let mut lab = lab();
    let default = Fake::serving(&lab.home, "instance-default", &["default"]);
    lab.follow_all();
    lab.looks(2);
    assert_eq!(lab.readers(), ["local"]);
    // A profile with a gateway of its own.
    let later = Fake::serving(&lab.home, "instance-later", &["later"]);
    later.asks("later", "s-later", "p1");
    until("the new profile's gateway is followed", || {
        lab.shown() == ["hermes:local/later:later:s-later:p1"]
    });
    assert_eq!(lab.readers(), ["local", "local/later"]);
    // A profile the gateway that runs starts to serve.
    default.serve(&["default", "added"]);
    default.asks("added", "s-added", "p2");
    let both = [
        "hermes:local/later:later:s-later:p1",
        "hermes:local:added:s-added:p2",
    ];
    until("the profile the gateway took on is polled", || {
        lab.shown() == both
    });
    assert_eq!(lab.readers(), ["local", "local/later"]);
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    assert!(default.world().mismatched.is_empty());
}

#[test]
fn stopping_one_of_two_gateways_keeps_its_cards_and_a_new_instance_closes_them_as_a_restart() {
    let mut lab = lab();
    let default = Fake::serving(&lab.home, "aaaaaaaa-default", &["default"]);
    let reports = Fake::serving(&lab.home, "bbbbbbbb-reports", &["reports"]);
    default.asks("default", "s-default", "p1");
    reports.asks("reports", "s-reports", "p2");
    lab.follow_all();
    let (kept, old) = (
        "hermes:local:default:s-default:p1",
        "hermes:local/reports:reports:s-reports:p2",
    );
    until("a card from each gateway shows", || {
        lab.shown() == [old, kept]
    });
    let card = |id: &str| lab.cards().into_iter().find(|card| card["id"] == id);
    let (untouched, stays) = (card(kept), card(old));
    let follower = lab.follower("local/reports");

    // The gateway stops. Its follower keeps trying, and its cards stay.
    reports.kill();
    until("the follower says its gateway is gone", || {
        !lab.said("local/reports connection failed").is_empty()
    });
    lab.looks(4);
    assert_eq!(lab.shown(), [old, kept]);
    assert_eq!(card(old), stays, "the card is the same card");
    assert_eq!(card(kept), untouched);
    assert!(Arc::ptr_eq(&follower, &lab.follower("local/reports")));
    assert!(follower.active());
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    assert!(lab.said("gateway gone").is_empty(), "{}", lab.log());

    // It starts again as a new instance with another question waiting:
    // what the old one asked did not survive it.
    let again = Fake::serving(&lab.home, "cccccccc-reports", &["reports"]);
    again.asks("reports", "s-again", "p3");
    let new = "hermes:local/reports:reports:s-again:p3";
    until("the new instance's card shows", || {
        lab.shown() == [new, kept]
    });
    assert_eq!(
        lab.said(&format!("close id={old}")),
        [format!("local/reports close id={old} why=GatewayRestarted")]
    );
    assert_eq!(
        card(kept),
        untouched,
        "the other gateway's card is untouched"
    );
    assert_eq!(lab.observer.as_ref().unwrap().ends(), ["local/reports"]);
    assert!(!follower.active(), "the old follower ended");
    assert!(!Arc::ptr_eq(&follower, &lab.follower("local/reports")));
    assert_eq!(lab.readers(), ["local", "local/reports"]);
    // One gateway going on under its id: nothing found, nothing gone.
    assert_eq!(lab.said("gateway found").len(), 1, "{}", lab.log());
    assert!(lab.said("gateway gone").is_empty(), "{}", lab.log());
}

#[test]
fn moving_a_profile_from_one_gateway_to_the_other_closes_its_cards_once_and_opens_them_once() {
    let mut lab = lab();
    let first = Fake::serving(&lab.home, "instance-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "instance-second", &["beta"]);
    first.asks("alpha", "s-alpha", "p1");
    first.asks("default", "s-default", "p2");
    second.asks("beta", "s-beta", "p3");
    lab.follow_all();
    let (was, now) = (
        "hermes:local:alpha:s-alpha:p1",
        "hermes:local/beta:alpha:s-alpha:p1",
    );
    let rest = [
        "hermes:local/beta:beta:s-beta:p3",
        "hermes:local:default:s-default:p2",
    ];
    until("three cards show", || {
        lab.shown() == [rest[0], was, rest[1]]
    });
    let card = |id: &str| lab.cards().into_iter().find(|card| card["id"] == id);
    let untouched = (card(rest[0]), card(rest[1]));
    let followers = lab.observer.as_ref().unwrap().followers();

    // The session and its question go with the profile.
    second.asks("alpha", "s-alpha", "p1");
    first.serve(&["default"]);
    second.serve(&["beta", "alpha"]);
    until(
        "the card shows under the gateway that serves it now",
        || lab.shown() == [now, rest[0], rest[1]],
    );
    lab.looks(3);
    // Closed under the old follower and opened under the new one, once
    // each, in that order.
    let lines = lab.said("s-alpha:p1");
    let expected = [
        format!("open id={was} kind=question session=hermes:alpha:s-alpha"),
        format!("close id={was} why=Retired"),
        format!("local/beta open id={now} kind=question session=hermes:alpha:s-alpha"),
    ];
    assert_eq!(lines, expected, "{}", lab.log());
    assert_eq!((card(rest[0]), card(rest[1])), untouched);
    // Both followers went on: no reader ended.
    for (was, is) in followers
        .iter()
        .zip(lab.observer.as_ref().unwrap().followers())
    {
        assert!(Arc::ptr_eq(was, &is));
    }
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    // The gateway that took the profile on was never asked for it before
    // it served it. The one that gave it up may refuse a poll that was
    // already on its way, which is what asks for the look.
    assert!(second.world().mismatched.is_empty(), "{}", lab.log());
    let refused = first.world().mismatched.clone();
    assert!(
        refused.iter().all(|profile| profile == "alpha"),
        "{refused:?}"
    );
}

#[test]
fn a_profile_two_instances_claim_gets_no_card_until_a_fresh_discovery_agrees() {
    let mut lab = lab();
    let first = Fake::serving(&lab.home, "instance-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "instance-second", &["beta"]);
    // The second gateway says it serves `alpha` too, as a restart that
    // overlaps would: both have a question waiting there. It is named by
    // `alpha`, the first of its profiles, and asked in the home of `beta`,
    // where its socket is.
    second.serve(&["beta", "alpha"]);
    first.asks("alpha", "s-alpha", "p1");
    second.asks("alpha", "s-alpha", "p1");
    first.asks("default", "s-default", "p2");
    second.asks("beta", "s-beta", "p3");
    lab.follow_all();
    let rest = [
        "hermes:local/alpha:beta:s-beta:p3",
        "hermes:local:default:s-default:p2",
    ];
    until("the cards of the other profiles show", || {
        lab.shown() == rest
    });
    lab.looks(4);
    // Both gateways are polled for it, and neither opens a card.
    until("both list the profile", || {
        listed(&first).contains(&"alpha".to_owned())
            && listed(&second).contains(&"alpha".to_owned())
    });
    lab.looks(2);
    assert_eq!(lab.shown(), rest, "{}", lab.log());
    assert_eq!(lab.said("contested"), ["contested profiles=[alpha]"]);

    // The first gateway lets go of it: one instance claims it now.
    first.serve(&["default"]);
    let now = "hermes:local/alpha:alpha:s-alpha:p1";
    until("the one that serves it opens its card", || {
        lab.shown() == [now, rest[0], rest[1]]
    });
    assert_eq!(lab.readers(), ["local", "local/alpha"]);
    assert_eq!(lab.said("s-alpha:p1").len(), 1, "{}", lab.log());
    let said = lab.said("contested");
    assert_eq!(
        said,
        ["contested profiles=[alpha]", "contested profiles=[]"]
    );
}
