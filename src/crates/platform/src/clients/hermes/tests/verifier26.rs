//! The first review of ticket 26 (`evidence/26/verifier-report.md`): the
//! verifier's five probes (`verifier-files/probes.rs`), which all failed
//! on `9f164fc`, tracked as tests.
//!
//! What is changed in them, each because the probe waited for the defect
//! itself. Every assertion stands, and some are added.
//!
//! - The two reclaim probes waited until the question showed under a new
//!   follower's name, and read that new card's deadline. They now wait for
//!   looks at the machine, and read the deadline of the card that was
//!   there. The newcomer is followed under a name of its own (the second
//!   review; `reverify26.rs`).
//! - The partial probe waited until the incomplete look had released the
//!   contested profile. It now waits for four looks.
use super::super::Shared;
use super::{
    fake::{home_of, Fake, Hold},
    lab, until,
};
use agent_hud_agents::{
    hermes::gateway::{GatewayId, Machine, Profile},
    parse_event,
};
use agent_hud_core::{AgentHudCore, SourceRoot};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

fn snapshot(core: &mut AgentHudCore) -> Value {
    core.request("snapshot", json!({})).unwrap()
}

fn needs(snapshot: &Value) -> Vec<Value> {
    let items = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = items.iter().filter(|item| item["kind"] == "needs-you");
    needs.cloned().collect()
}

/// A gateway keeps the name of an anchor profile it serves no more, and
/// another gateway then serves that profile: the first, which still
/// runs, keeps its follower, its name and its cards as they are.
fn reclaims(dismiss: bool) {
    let mut lab = lab();
    let mut core = AgentHudCore::new(vec![], lab.data.join("core"));
    core.request("set_setting", json!({"key":"sound", "value":true}))
        .unwrap();
    snapshot(&mut core);
    lab.requests = core.requests();
    let _default = Fake::serving(&lab.home, "dddddddd-default", &["default"]);
    let running = Fake::serving(&lab.home, "aaaaaaaa-still-running", &["alpha", "beta"]);
    lab.follow_all();
    lab.looks(3);
    // Sound is primed before the question arrives.
    snapshot(&mut core);
    running.asks("beta", "s-beta", "p2");
    let old = "hermes:local/alpha:beta:s-beta:p2";
    until("the old follower's question shows", || lab.shown() == [old]);
    let original = lab.cards()[0].clone();
    let deadline = lab.requests.deadline(old).unwrap();
    let before = needs(&snapshot(&mut core));
    assert_eq!(before.len(), 1, "sound control");
    let follower = lab.follower("local/alpha");
    if dismiss {
        let leave = json!({"kind":"leave"});
        lab.requests
            .answer(old, &leave, crate::hooks::now_ms())
            .unwrap();
        until("the dismissed card stays hidden", || lab.cards().is_empty());
    }
    running.serve(&["beta"]);
    until("the retired anchor socket is gone", || {
        !home_of(&lab.home, "alpha").join("gateway.sock").exists()
    });
    lab.looks(3);
    assert!(
        follower.active(),
        "control: retained its name after retirement"
    );
    let newcomer = Fake::serving(&lab.home, "zzzzzzzz-new-alpha", &["alpha"]);
    lab.looks(3);
    lab.looks(3);
    let after = needs(&snapshot(&mut core));
    let ends = lab.observer.as_ref().unwrap().ends();
    eprintln!(
        "RECLAIM dismiss={dismiss} old_active={} shown={:?} sounds_before={} sounds_after={} ends={ends:?}",
        follower.active(),
        lab.shown(),
        before.len(),
        after.len(),
    );
    eprintln!("RECLAIM LOG\n{}", lab.log());
    assert!(running.world().calls("clarify.respond").is_empty());
    assert!(newcomer.world().calls("clarify.respond").is_empty());
    assert!(
        follower.active(),
        "a live gateway's follower must keep its identity"
    );
    assert!(Arc::ptr_eq(&follower, &lab.follower("local/alpha")));
    assert!(ends.is_empty(), "{ends:?}");
    // The newcomer is followed too, under a name of its own.
    let alpha = Profile::named("alpha").unwrap();
    let apart = GatewayId::apart(Machine::This, alpha, "zzzzzzzz-new-alpha").to_string();
    assert_eq!(lab.readers(), ["local", "local/alpha", apart.as_str()]);
    if dismiss {
        assert!(
            lab.cards().is_empty(),
            "a dismissed question of a still-running gateway was resurrected"
        );
        assert_eq!(after.len(), before.len());
    } else {
        assert_eq!(
            after.len(),
            before.len(),
            "the same question sounded twice solely because its gateway lost its old anchor name"
        );
        assert_eq!(lab.shown(), [old]);
        assert_eq!(lab.requests.deadline(old), Some(deadline));
        assert_eq!(lab.cards()[0], original, "the same card, as registered");
    }
}

#[test]
fn verifier26_a_live_gateway_reclaim_keeps_cards_deadlines_and_one_sound() {
    reclaims(false);
}

#[test]
fn verifier26_a_live_gateway_reclaim_keeps_a_dismissed_card_dismissed() {
    reclaims(true);
}

#[test]
fn verifier26_a_partial_discovery_does_not_release_a_contested_profile() {
    let mut lab = lab();
    let first = Fake::serving(&lab.home, "aaaaaaaa-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    second.serve(&["beta", "alpha"]);
    first.asks("alpha", "s-alpha", "p1");
    second.asks("alpha", "s-alpha", "p1");
    first.asks("default", "s-default", "p2");
    second.asks("beta", "s-beta", "p3");
    lab.follow_all();
    until("both uncontested cards show", || lab.cards().len() == 2);
    lab.looks(4);
    let before = lab.shown();
    let followers = lab.observer.as_ref().unwrap().followers();
    let alpha = Profile::named("alpha").unwrap();
    let withheld = |alpha: &Profile| {
        let held = |follower: &Arc<Shared>| follower.model().gateway.withheld().contains(alpha);
        followers.iter().all(held)
    };
    assert!(withheld(&alpha));
    // Only the control reply times out. Both gateways and their existing
    // polling sockets remain live, and both still claim alpha.
    *lab.route.slow.lock().unwrap() = vec!["beta".to_owned()];
    lab.looks(4);
    let after = lab.shown();
    eprintln!(
        "PARTIAL before={before:?} after={after:?} first_serves={:?} second_serves={:?}",
        first.world().serves,
        second.world().serves
    );
    eprintln!("PARTIAL LOG\n{}", lab.log());
    assert_eq!(
        after, before,
        "an incomplete discovery must preserve the contested set and cards"
    );
    // The contested set is what it was, nothing was retired from the
    // claimant that did not answer, and both still hold the card back.
    assert_eq!(lab.said("contested"), ["contested profiles=[alpha]"]);
    assert!(lab.said("retired").is_empty(), "{}", lab.log());
    assert!(withheld(&alpha));
    let holds = |reader: &str| {
        lab.follower(reader)
            .model()
            .gateway
            .holds()
            .contains(&alpha)
    };
    assert!(holds("local") && holds("local/alpha"));
    // Only a whole look resolves it: every home answers again, and the
    // first gateway has let go of the profile.
    lab.route.slow.lock().unwrap().clear();
    first.serve(&["default"]);
    let now = "hermes:local/alpha:alpha:s-alpha:p1";
    until(
        "a whole look frees the profile for the one that serves it",
        || lab.shown().contains(&now.to_owned()),
    );
    assert_eq!(lab.shown().len(), 3);
    assert_eq!(lab.said("s-alpha:p1").len(), 1, "{}", lab.log());
}

#[test]
fn verifier26_a_custom_root_card_matches_its_stored_session_and_hook() {
    let mut lab = lab();
    lab.home = lab.home.join("profiles/work");
    crate::os::private_directory(&lab.home).unwrap();
    lab.route = Arc::new(super::route::Way::to(&lab.home));
    let path = lab.home.join("state.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    let schema = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agents/tests/fixtures/hermes/macos/state.schema.sql");
    db.execute_batch(&std::fs::read_to_string(schema).unwrap())
        .unwrap();
    let started = (crate::hooks::now_ms() - 600_000) as f64 / 1000.0;
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES('s1','cron','m',?1,?1)",
        [started],
    )
    .unwrap();
    drop(db);
    let provider = "hermes";
    let source = SourceRoot { path, provider };
    let mut core = AgentHudCore::new(vec![source], lab.data.join("core"));
    lab.requests = core.requests();
    core.request("refresh", json!({})).unwrap();
    until("the stored row is collected", || {
        let rows = snapshot(&mut core)["sessions"].clone();
        let stored = |row: &Value| row["id"] == "hermes:work:s1";
        rows.as_array().unwrap().iter().any(stored)
    });
    let gateway = Fake::serving(&lab.home, "aaaaaaaa-custom", &["default"]);
    gateway.asks("default", "s1", "p1");
    lab.follow_all();
    until("the root question shows", || lab.cards().len() == 1);
    let card = lab.cards()[0].clone();
    let mut payload = json!({"hook_event_name":"pre_llm_call", "session_id":"s1"});
    agent_hud_agents::hooks::add_environment("hermes", &mut payload, &|key| {
        (key == "HERMES_HOME").then(|| lab.home.to_string_lossy().into_owned())
    });
    let now = crate::hooks::now_ms();
    let hook = parse_event("hermes", "--completion-hook", &payload, now);
    core.ingest_hook_events(vec![hook.unwrap().unwrap()]);
    let current = snapshot(&mut core);
    let rows: Vec<_> = current["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| json!({"id":row["id"],"state":row["state"],"requests":row["liveFacts"]["requests"]}))
        .collect();
    eprintln!(
        "CUSTOM card_session={} waiting={:?} rows={rows:?}",
        card["sessionId"],
        lab.waiting()
    );
    assert_eq!(
        card["sessionId"], "hermes:work:s1",
        "gateway card must match the stored reader and hook's session id"
    );
    // One row, the stored one, and the gateway was asked for the profile
    // by the name Hermes calls it, which no call was refused for.
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(card["id"], "hermes:local:work:s1:p1");
    assert_eq!(lab.waiting(), ["hermes:work:s1"]);
    assert!(gateway.world().mismatched.is_empty());
    assert_eq!(lab.readers(), ["local"]);
}

#[test]
fn verifier26_a_poll_waiting_on_a_follower_stops_before_its_next_round() {
    let mut lab = lab();
    let _default = Fake::serving(&lab.home, "aaaaaaaa-default", &["default"]);
    let reports = Fake::serving(&lab.home, "bbbbbbbb-reports", &["reports"]);
    reports.asks("reports", "s-reports", "p1");
    lab.follow_all();
    until("the reports card shows", || lab.cards().len() == 1);
    let follower = lab.follower("local/reports");
    let held = Arc::new(Hold::default());
    reports.world().hold_next = Some(("session.active_list", held.clone()));
    until("the reports poll is waiting on its list reply", || {
        held.entered.load(Ordering::Acquire)
    });
    let at_stop = reports.world().received.len();
    lab.observer.take();
    assert!(!follower.active());
    held.released.store(true, Ordering::Release);
    until("the stopped follower's workers returned", || {
        Arc::strong_count(&follower) == 1
    });
    let world = reports.world();
    let afterwards: Vec<_> = (world.received.iter().skip(at_stop))
        .map(|(_, frame)| frame["method"].clone())
        .collect();
    eprintln!("POLL STOP calls_after_stop={afterwards:?}");
    assert!(
        afterwards.iter().all(|method| method == "session.detach"),
        "a stopped follower began another poll round after its pending reply returned"
    );
}
