//! The second review of ticket 26 (`evidence/26/verifier-report-2.md`):
//! its two probes (`verifier-files-2/reverify26.rs`), which failed on
//! `7946c66`, and its run against a real Hermes 0.22.5 gateway in a home
//! at `…/profiles/work`, as a test against the pretend gateway, which
//! now names a profile as Hermes does.
//!
//! What is changed in the two probes, each because the probe waited for
//! the defect itself. Every assertion stands.
//!
//! - The newcomer probe waited for the log to say that the newcomer
//!   waits. It now waits for four looks at the machine.
//! - The socket probe waited for the contested set to change and for the
//!   held question to open. It now waits for six looks.
use super::super::socket::{Reply, Socket};
use super::{
    fake::{home_of, Fake},
    lab, until,
};
use agent_hud_agents::hermes::gateway::{
    active_list, granted, identify, identify_line, refusal, ticket_line, GatewayId, Identified,
    Machine, Profile, Refusal, Scope,
};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
fn reverify26_a_waiting_newcomer_must_receive_a_card_within_a_bound() {
    let mut lab = lab();
    let _default = Fake::serving(&lab.home, "dddddddd-default", &["default"]);
    let running = Fake::serving(&lab.home, "aaaaaaaa-still-running", &["alpha", "beta"]);
    running.asks("beta", "s-beta", "p2");
    lab.follow_all();
    until("beta card", || lab.cards().len() == 1);
    let follower = lab.follower("local/alpha");
    let kept = lab.cards()[0].clone();
    running.serve(&["beta"]);
    until("alpha socket retired", || {
        !home_of(&lab.home, "alpha").join("gateway.sock").exists()
    });
    lab.looks(4);
    let newcomer = Fake::serving(&lab.home, "zzzzzzzz-new-alpha", &["alpha"]);
    newcomer.asks("alpha", "s-alpha", "p9");
    lab.looks(4);
    let started = Instant::now();
    let before = lab.route.asked().homes;
    while started.elapsed() < Duration::from_secs(2) {
        lab.looks(4);
    }
    let after = lab.route.asked().homes;
    let shown = |lab: &super::Lab| lab.shown().iter().any(|id| id.contains("alpha:s-alpha:p9"));
    let displayed = shown(&lab);
    let (serves, polls) = {
        let world = newcomer.world();
        let polls = world.calls("session.active_list").len();
        (world.serves.clone(), polls)
    };
    eprintln!(
        "RESIDUAL elapsed={:?} home_inventories={} old_active={} readers={:?} cards={:?} newcomer_serves={serves:?} newcomer_poll_calls={polls} log=\n{}",
        started.elapsed(),
        after - before,
        follower.active(),
        lab.readers(),
        lab.shown(),
        lab.log()
    );
    assert!(
        follower.active(),
        "control: the existing follower stays alive"
    );
    // The newcomer has a name of its own: the anchor it would have had,
    // told apart by its instance. The old follower's name is as it was.
    let alpha = Profile::named("alpha").unwrap();
    let apart = GatewayId::apart(Machine::This, alpha, "zzzzzzzz-new-alpha").to_string();
    assert_eq!(lab.readers(), ["local", "local/alpha", apart.as_str()]);
    assert!(Arc::ptr_eq(&follower, &lab.follower("local/alpha")));
    let card = format!("hermes:{apart}:alpha:s-alpha:p9");
    assert_eq!(lab.shown(), ["hermes:local/alpha:beta:s-beta:p2", &card]);
    let old = |card: &Value| card["id"] == "hermes:local/alpha:beta:s-beta:p2";
    assert_eq!(lab.cards().into_iter().find(old), Some(kept));
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    // Stopping the old gateway changes nothing for either: its follower
    // and its card stay, and the newcomer's question still shows.
    running.kill();
    lab.looks(15);
    let still_missing = !shown(&lab);
    let polls = newcomer.world().calls("session.active_list").len();
    eprintln!(
        "RESIDUAL_STOPPED old_gateway_socket_exists={} old_follower_active={} newcomer_missing={still_missing} newcomer_poll_calls={polls} readers={:?} cards={:?}",
        home_of(&lab.home, "beta").join("gateway.sock").exists(),
        follower.active(),
        lab.readers(),
        lab.shown()
    );
    assert!(
        displayed,
        "a repeatedly verified ready newcomer still has no card and has no waiting deadline"
    );
    assert!(follower.active() && !still_missing);
    assert_eq!(lab.shown().len(), 2);
}

#[test]
fn reverify26_unlinking_a_live_claimants_control_socket_must_not_clear_a_contest() {
    let mut lab = lab();
    let first = Fake::serving(&lab.home, "aaaaaaaa-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    second.serve(&["beta", "alpha"]);
    first.asks("alpha", "s-alpha", "p1");
    second.asks("alpha", "s-alpha", "p1");
    first.asks("default", "s-default", "p2");
    second.asks("beta", "s-beta", "p3");
    lab.follow_all();
    until("two uncontested cards", || lab.cards().len() == 2);
    until("verified contest", || {
        lab.said("contested") == ["contested profiles=[alpha]"]
    });
    let before = lab.shown();
    let follower = lab.follower("local/alpha");
    // Only the pathname is removed. Its Unix listener, WebSocket server,
    // established polling socket, instance id and served set remain live.
    std::fs::remove_file(home_of(&lab.home, "beta").join("gateway.sock")).unwrap();
    lab.looks(6);
    let (serves, polls) = {
        let world = second.world();
        let polls = world.calls("session.active_list").len();
        (world.serves.clone(), polls)
    };
    eprintln!(
        "NO_SOCKET before={before:?} after={:?} old_follower_active={} first_serves={:?} second_serves={serves:?} second_polls={polls} log=\n{}",
        lab.shown(),
        follower.active(),
        first.world().serves,
        lab.log()
    );
    assert_eq!(
        lab.shown(),
        before,
        "absence of a live claimant's control socket is not agreement that it relinquished alpha"
    );
    // The contest is what it was, and nothing was taken from the claimant
    // that could not be asked.
    assert_eq!(lab.said("contested"), ["contested profiles=[alpha]"]);
    assert!(lab.said("retired").is_empty(), "{}", lab.log());
    let alpha = Profile::named("alpha").unwrap();
    assert!(follower.active() && follower.model().gateway.holds().contains(&alpha));
}

/// The reply to one call on `socket`.
fn call(socket: &mut Socket, profile: &Profile) -> Value {
    let asked = socket.send(&active_list(profile, Scope::Host)).unwrap();
    match socket.reply(asked, Duration::from_secs(5), &mut |_| {}) {
        Ok(Reply::Frame(reply)) => reply,
        _ => panic!("no reply"),
    }
}

/// The second review ran a real Hermes 0.22.5 gateway in a home at
/// `…/profiles/work` (`verifier-files-2/real-gateway-probe.log`): it
/// refused a call for `default` as `profile_mismatch`, took one for
/// `work`, and its question got no card, because the calls were routed
/// to `default`. Here the pretend gateway names a profile as Hermes does,
/// and the same three things are looked at.
#[test]
fn reverify26_a_home_in_a_profiles_folder_is_routed_by_the_name_real_hermes_takes() {
    let mut lab = lab();
    lab.home = lab.home.join("profiles/work");
    crate::os::private_directory(&lab.home).unwrap();
    lab.route = Arc::new(super::route::Way::to(&lab.home));
    let gateway = Fake::serving(&lab.home, "aaaaaaaa-custom", &["default"]);
    assert_eq!(gateway.named("default"), "work");

    // On a socket of its own, as the review asked the real gateway.
    let wait = Duration::from_secs(5);
    let told = super::ask(&lab.home, &identify_line(), wait).unwrap();
    let Identified::Ready(instance) = identify(&told, lab.home.to_str().unwrap()) else {
        panic!("not ready")
    };
    let grant = super::ask(&lab.home, &ticket_line(&instance, Scope::Host), wait).unwrap();
    let ticket = granted(&grant, &instance).unwrap();
    let mut socket = Socket::open(&*lab.route, &instance, ticket).unwrap();
    let refused = call(&mut socket, &Profile::Default);
    assert_eq!(refusal(&refused), Some(Refusal::ProfileMismatch));
    let work = Profile::named("work").unwrap();
    let listed = call(&mut socket, &work);
    assert_eq!(listed["result"]["sessions"], serde_json::json!([]));
    socket.close();
    let mismatched = gateway.world().mismatched.clone();
    assert_eq!(mismatched, ["default"], "the one call made here by hand");

    // The question of that home gets its card, and every call the
    // follower makes names the profile as Hermes does.
    gateway.asks("default", "s1", "p1");
    lab.follow_all();
    until("the question of the home shows", || lab.cards().len() == 1);
    lab.looks(3);
    let card = lab.cards()[0].clone();
    assert_eq!(card["id"], "hermes:local:work:s1:p1");
    assert_eq!(card["sessionId"], "hermes:work:s1");
    assert_eq!(lab.readers(), ["local"]);
    let world = gateway.world();
    let routed: Vec<Value> = (world.received.iter())
        .filter_map(|(_, frame)| frame["params"].get("profile").cloned())
        .collect();
    assert!(!routed.is_empty() && routed.iter().skip(2).all(|name| name == "work"));
    assert_eq!(
        world.mismatched,
        ["default"],
        "no call of the follower was refused"
    );
    assert!(!lab.log().contains("profile_mismatch"));
}
