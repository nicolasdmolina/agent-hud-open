//! What each follower tells core, and how one ends (ticket 26). Every
//! follower tells under its own name, so a stalled one lapses alone and a
//! poll by one says nothing of another's sessions. A follower keeps its
//! id for as long as it lives, and when it ends, its reader ends in core
//! exactly once and before anything takes its place.
use super::super::{follower::End, Pace};
use super::{
    fake::{Fake, Hold},
    gateways::looking,
    lab, until, Lab,
};
use agent_hud_agents::{hermes::AGENT, parse_event, LiveState};
use agent_hud_core::{requests::LIVE_STATES_MS, AgentHudCore, SourceRoot};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

/// When `reader` last read, as core holds it.
fn read_ms(lab: &Lab, reader: &str) -> Option<i64> {
    let all = lab.requests.native_states();
    all.of(AGENT.id, reader).map(|reading| reading.read_ms)
}

/// The sessions `reader` told core of, with what each does.
fn told(lab: &Lab, reader: &str) -> Vec<(String, LiveState)> {
    let all = lab.requests.native_states();
    let reading = all.of(AGENT.id, reader);
    let mut sessions: Vec<_> = reading
        .into_iter()
        .flat_map(|r| r.sessions.clone())
        .collect();
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    sessions
}

/// Holds the gateway's next list of sessions, so that the poll that asked
/// for it does not come back: a follower that stalls.
fn stall(gateway: &Fake) -> Arc<Hold> {
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("session.active_list", held.clone()));
    until("the gateway holds its reply", || {
        held.entered.load(Ordering::Acquire)
    });
    held
}

/// The tests' pace with a poll that waits long for its answers: a stalled
/// gateway stalls its follower, where a short wait would fail the poll.
fn patient() -> Pace {
    Pace {
        answer: Duration::from_secs(30),
        ..looking()
    }
}

/// One gateway for the default profile and one for `reports`, each with
/// a session in a turn and another that waits on a question.
fn two(lab: &mut Lab, pace: Pace) -> (Fake, Fake) {
    let default = Fake::serving(&lab.home, "aaaaaaaa-default", &["default"]);
    let reports = Fake::serving(&lab.home, "bbbbbbbb-reports", &["reports"]);
    default.runs("default", "d-runs", true);
    default.asks("default", "d-asks", "p1");
    reports.runs("reports", "r-runs", true);
    reports.asks("reports", "r-asks", "p2");
    lab.follow_at(pace);
    until("both gateways are read", || {
        told(lab, "local").len() == 2 && told(lab, "local/reports").len() == 2
    });
    until("both cards show", || lab.shown().len() == 2);
    (default, reports)
}

#[test]
fn a_follower_stalled_for_over_thirty_seconds_lapses_alone_and_the_cards_of_both_stay() {
    let mut lab = lab();
    let (_default, reports) = two(&mut lab, patient());
    let cards = lab.cards();
    let held = stall(&reports);
    let stalled = read_ms(&lab, "local/reports").unwrap();
    until("the other follower reads on", || {
        read_ms(&lab, "local").is_some_and(|at| at > stalled)
    });
    let state = |id: &str, at: i64| {
        let all = lab.requests.native_states();
        all.told(AGENT.id, id, at).map(|(state, _)| state)
    };
    let (stale, fresh) = ("hermes:reports:r-runs", "hermes:d-runs");
    let lapses = stalled + LIVE_STATES_MS;
    // Up to thirty seconds after its last poll, what it read is believed.
    assert_eq!(state(stale, lapses - 1), Some(LiveState::Running));
    // Core lapses it by itself: the stalled follower says nothing more.
    assert_eq!(lab.requests.next_deadline(), Some(lapses));
    lab.requests.expire(lapses);
    assert_eq!(
        state(stale, lapses),
        None,
        "its sessions lost their live state"
    );
    assert_eq!(read_ms(&lab, "local/reports"), None);
    // The other follower's sessions keep theirs, at that same moment.
    assert_eq!(state(fresh, lapses), Some(LiveState::Running));
    assert!(read_ms(&lab, "local").is_some_and(|at| at > stalled));
    // The cards of both stay, and their sessions go on waiting.
    assert_eq!(lab.cards(), cards);
    assert_eq!(lab.waiting(), ["hermes:d-asks", "hermes:reports:r-asks"]);
    assert_eq!(lab.readers(), ["local", "local/reports"]);
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    held.released.store(true, Ordering::Release);
    // It reads again, and is believed again.
    until("the stalled follower reads again", || {
        read_ms(&lab, "local/reports").is_some_and(|at| at > stalled)
    });
    assert_eq!(told(&lab, "local/reports").len(), 2);
}

/// A core that reads the `state.db` of the lab's Hermes home, which holds
/// the session `s1` of the default profile, started ten minutes ago.
fn core(lab: &Lab) -> AgentHudCore {
    let schema = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agents/tests/fixtures/hermes/macos/state.schema.sql");
    let path = lab.home.join("state.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(&std::fs::read_to_string(schema).unwrap())
        .unwrap();
    let started = (crate::hooks::now_ms() - 600_000) as f64 / 1000.0;
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES('s1','cron','m',?1,?1)",
        rusqlite::params![started],
    )
    .unwrap();
    let provider = "hermes";
    AgentHudCore::new(vec![SourceRoot { path, provider }], lab.data.join("hud"))
}

/// What a fresh snapshot of `core` says the session does.
fn state(core: &mut AgentHudCore, id: &str) -> Option<String> {
    let mut snapshot = core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        let row = |row: &&Value| row["id"] == id;
        let rows = snapshot["sessions"].as_array();
        if let Some(row) = rows.and_then(|rows| rows.iter().find(row)) {
            return row["state"].as_str().map(str::to_owned);
        }
        std::thread::sleep(Duration::from_millis(5));
        snapshot = core.request("snapshot", json!({})).unwrap();
    }
    None
}

/// Ticket 24's first regression, end to end: the follower of one gateway
/// reads a session in a turn, the session's completion hook arrives, and
/// the follower of the other gateway polls afterwards. That later poll
/// says nothing of the session, which shows as finished.
#[test]
fn a_completion_hook_for_one_gateways_session_stands_when_the_other_follower_polls_later() {
    let mut lab = lab();
    let mut core = core(&lab);
    lab.requests = core.requests();
    let default = Fake::serving(&lab.home, "aaaaaaaa-default", &["default"]);
    let reports = Fake::serving(&lab.home, "bbbbbbbb-reports", &["reports"]);
    default.runs("default", "s1", true);
    reports.runs("reports", "r1", true);
    lab.follow_at(patient());
    let is =
        |core: &mut AgentHudCore, what: &str| state(core, "hermes:s1").as_deref() == Some(what);
    until("the session shows as running", || is(&mut core, "running"));

    // Its own follower reads no more for now. The other reads on, and the
    // session keeps the reading of the follower that lists it.
    let held = stall(&default);
    let read = read_ms(&lab, "local").unwrap();
    let later = |lab: &Lab, than: i64| read_ms(lab, "local/reports").is_some_and(|at| at > than);
    until("the other follower polls", || later(&lab, read));
    assert!(
        is(&mut core, "running"),
        "{:?}",
        state(&mut core, "hermes:s1")
    );

    // The turn ends: the hook is newer than the reading that lists it.
    let ended = crate::hooks::now_ms();
    let payload = json!({"hook_event_name": "on_session_end", "session_id": "s1", "cwd": "/tmp",
        "extra": {"turn_id": "s1:turn-1", "surface": "cli"}});
    let hook = parse_event("hermes", "--completion-hook", &payload, ended);
    core.ingest_hook_events(vec![hook.unwrap().unwrap()]);
    assert!(is(&mut core, "idle"), "{:?}", state(&mut core, "hermes:s1"));

    // The other follower polls after the hook, and again: no news of it.
    for _ in 0..2 {
        let last = read_ms(&lab, "local/reports").unwrap().max(ended);
        until("the other follower polls after the hook", || {
            later(&lab, last)
        });
        assert!(is(&mut core, "idle"), "it follows its hook");
    }
    assert_eq!(
        read_ms(&lab, "local"),
        Some(read),
        "its own follower read nothing since"
    );
    assert!(read_ms(&lab, "local/reports").unwrap() > ended);

    // Its own follower reads again after the hook, and stands again.
    held.released.store(true, Ordering::Release);
    until("its own follower reads again", || {
        read_ms(&lab, "local").is_some_and(|at| at > ended)
    });
    until("the session shows as running again", || {
        is(&mut core, "running")
    });
}

#[test]
fn the_anchor_profile_is_retired_from_a_running_gateway() {
    let mut lab = lab();
    let gateway = Fake::serving(&lab.home, "aaaaaaaa-two", &["alpha", "beta"]);
    gateway.asks("alpha", "s-alpha", "p1");
    gateway.asks("beta", "s-beta", "p2");
    lab.follow_all();
    let (anchor, other) = (
        "hermes:local/alpha:alpha:s-alpha:p1",
        "hermes:local/alpha:beta:s-beta:p2",
    );
    until("a card from each profile shows", || {
        lab.shown() == [anchor, other]
    });
    let follower = lab.follower("local/alpha");
    let card = |id: &str| lab.cards().into_iter().find(|card| card["id"] == id);
    let kept = card(other);

    // The gateway goes on and serves `alpha` no more.
    gateway.serve(&["beta"]);
    until("the anchor profile's card closes", || {
        lab.shown() == [other]
    });
    assert_eq!(
        lab.said(&format!("close id={anchor}")),
        [format!("local/alpha close id={anchor} why=Retired")]
    );
    // The follower keeps its id: it is the same follower, under the same
    // name, and the card of its other profile is the same card.
    lab.looks(3);
    assert!(Arc::ptr_eq(&follower, &lab.follower("local/alpha")));
    assert!(follower.active());
    assert_eq!(lab.readers(), ["local", "local/alpha"]);
    assert_eq!(card(other), kept, "the same request, as it was registered");
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    assert_eq!(lab.said("asks profile"), ["local/alpha asks profile=beta"]);
    assert_eq!(lab.waiting(), ["hermes:beta:s-beta"]);

    // What its other profile asks next is named by the same id, and is
    // answered through the home the follower asks now.
    gateway.asks("beta", "s-beta-2", "p3");
    let next = "hermes:local/alpha:beta:s-beta-2:p3";
    until("the next question shows", || lab.shown() == [next, other]);
    let decision = json!({"kind": "answer", "answers": {"p3": {"selected": ["one"]}}});
    lab.requests
        .answer(next, &decision, crate::hooks::now_ms())
        .unwrap();
    assert_eq!(lab.delivery(next)["state"], "confirmed");
    assert!(gateway
        .world()
        .mismatched
        .iter()
        .all(|profile| profile == "alpha"));

    // The anchor is picked again only for a follower made after a restart.
    gateway.kill();
    let again = Fake::serving(&lab.home, "bbbbbbbb-two", &["beta"]);
    again.asks("beta", "s-beta-3", "p4");
    let new = "hermes:local/beta:beta:s-beta-3:p4";
    until("the new instance is followed", || lab.shown() == [new]);
    assert_eq!(lab.readers(), ["local", "local/beta"]);
    assert_eq!(lab.observer.as_ref().unwrap().ends(), ["local/alpha"]);
    assert!(!follower.active());
}

/// Brings the pretend gateway back as another instance with one session
/// in a turn and one that waits on a question.
fn restart(gateway: &Fake, instance: &str) {
    {
        let mut world = gateway.world();
        world.identify.as_mut().unwrap()["result"]["instance_id"] = json!(instance);
        world.instance = instance.to_owned();
        world.streams.clear();
    }
    gateway.runs("default", "runs-again", true);
    gateway.asks("default", "asks-again", "p2");
    gateway.cut();
}

#[test]
fn a_late_end_does_not_clear_the_replacement() {
    let mut lab = lab();
    let gateway = Fake::serving(&lab.home, "aaaaaaaa-first", &["default"]);
    gateway.runs("default", "runs", true);
    gateway.asks("default", "asks", "p1");
    lab.follow_all();
    until("the first instance's card shows", || {
        lab.shown() == ["hermes:local:default:asks:p1"]
    });
    let old = lab.follower("local");

    // The gateway restarts: the follower ends and one with the same id
    // takes its place.
    restart(&gateway, "bbbbbbbb-second");
    let new = "hermes:local:default:asks-again:p2";
    until("the replacement shows its card", || lab.shown() == [new]);
    let running = vec![
        ("hermes:asks-again".to_owned(), LiveState::Waiting),
        ("hermes:runs-again".to_owned(), LiveState::Running),
    ];
    until("the replacement tells its states", || {
        told(&lab, "local") == running
    });
    let replacement = lab.follower("local");
    assert!(!Arc::ptr_eq(&old, &replacement));
    assert!(!old.active());
    assert_eq!(lab.observer.as_ref().unwrap().ends(), ["local"]);

    // The old follower's end arrives again, late, in every form.
    let (cards, flags) = (lab.cards(), lab.waiting());
    old.end(End::Restarted);
    old.end(End::Replaced);
    old.end(End::Off);
    old.stop();
    // The replacement's states, flags and cards are all still in core.
    assert_eq!(told(&lab, "local"), running);
    assert_eq!(lab.waiting(), flags);
    assert_eq!(flags, ["hermes:asks-again"]);
    assert_eq!(lab.cards(), cards);
    assert_eq!(lab.shown(), [new]);
    assert_eq!(
        lab.observer.as_ref().unwrap().ends(),
        ["local"],
        "ended once"
    );
    // And it goes on reading under the name they share.
    let read = read_ms(&lab, "local").unwrap();
    until("the replacement reads on", || {
        read_ms(&lab, "local").is_some_and(|at| at > read)
    });
    assert!(replacement.active());
}

#[test]
fn a_reader_ends_once() {
    let mut lab = lab();
    let (_default, reports) = two(&mut lab, looking());
    let observer = |lab: &Lab| lab.observer.as_ref().unwrap().ends();
    let mut made = lab.observer.as_ref().unwrap().followers();
    assert!(observer(&lab).is_empty(), "no follower ended yet");

    // One gateway restarts: its follower ends, and nobody else's.
    reports.kill();
    let again = Fake::serving(&lab.home, "cccccccc-reports", &["reports"]);
    again.asks("reports", "r-again", "p3");
    until("the new instance's card shows", || {
        lab.shown()
            .contains(&"hermes:local/reports:reports:r-again:p3".to_owned())
    });
    assert_eq!(observer(&lab), ["local/reports"]);
    made.push(lab.follower("local/reports"));
    // The reader that ended and the one that began share a name: what
    // core holds under it is the replacement's.
    until("the replacement is read", || {
        told(&lab, "local/reports") == [("hermes:reports:r-again".to_owned(), LiveState::Waiting)]
    });

    // Client hooks turn off: every follower that is left ends, once.
    let stopped = lab.observer.take().unwrap();
    stopped.all.stop();
    let mut ends = stopped.ends();
    ends.sort();
    assert_eq!(ends, ["local", "local/reports", "local/reports"]);
    assert_eq!(made.len(), 3, "one end for each follower there ever was");
    assert!(lab.requests.native_states().is_empty());
    assert!(lab.waiting().is_empty());
    assert!(lab.cards().is_empty());
    // An end that comes again, to any of them, is not counted again.
    for follower in &made {
        assert!(!follower.active());
        follower.end(End::Off);
        follower.end(End::Restarted);
    }
    assert_eq!(stopped.ends().len(), 3);
}

/// Answers the question on `id` with its first choice, while the gateway
/// holds its reply: the answer is out, and what became of it is not known.
/// No poll comes back meanwhile to say the request settled, so the card
/// shows until something else closes it.
fn answer_held(lab: &Lab, gateway: &Fake, id: &str, prompt: &str) -> Arc<Hold> {
    let polls = stall(gateway);
    let held = Arc::new(Hold::default());
    gateway.world().hold_too = Some(("clarify.respond", held.clone()));
    // The held poll is let go when the gateway's fifteen seconds are up.
    drop(polls);
    let decision = json!({"kind": "answer", "answers": {prompt: {"selected": ["one"]}}});
    lab.requests
        .answer(id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the gateway holds its reply to the answer", || {
        held.entered.load(Ordering::Acquire)
    });
    held
}

/// Ticket 13's receipt rule, for a follower that ends with its gateway:
/// an answer that was written before the end keeps the receipt Hermes's
/// reply gives it, as it did when one follower lived through a restart.
#[test]
fn an_answer_that_is_out_when_its_follower_ends_as_restarted_keeps_the_receipt_hermes_gives() {
    let mut lab = lab();
    let (_default, reports) = two(&mut lab, patient());
    let id = "hermes:local/reports:reports:r-asks:p2";
    let held = answer_held(&lab, &reports, id, "p2");
    let follower = lab.follower("local/reports");
    follower.end(End::Restarted);
    assert_eq!(
        lab.said(&format!("close id={id}")),
        [format!("local/reports close id={id} why=GatewayRestarted")]
    );
    assert!(!lab.shown().contains(&id.to_owned()));
    // Hermes took the answer, and says so after the follower ended.
    held.released.store(true, Ordering::Release);
    assert_eq!(lab.delivery(id)["state"], "confirmed");
    // Nothing more went to the gateway for it: one answer, sent once.
    assert_eq!(reports.world().calls("clarify.respond").len(), 1);
}

/// And for one that ends because client hooks turned off: nothing is
/// told of the answer, as before.
#[test]
fn an_answer_that_is_out_when_client_hooks_turn_off_gets_no_receipt() {
    let mut lab = lab();
    let (default, _reports) = two(&mut lab, patient());
    let id = "hermes:local:default:d-asks:p1";
    let held = answer_held(&lab, &default, id, "p1");
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    until("the gateway is done with the answer's socket", || {
        default.world().visit_over()
    });
    std::thread::sleep(Duration::from_millis(100));
    let rows = lab.requests.deliveries(crate::hooks::now_ms());
    assert!(rows.iter().all(|row| row["id"] != id), "{rows:?}");
    assert!(lab.cards().is_empty());
}

/// What every gateway and the route were asked so far.
fn asked(lab: &Lab, gateways: &[&Fake]) -> Vec<usize> {
    let route = lab.route.asked();
    let mut counts = vec![route.homes, route.verbs.len(), route.dials.len()];
    for gateway in gateways {
        let world = gateway.world();
        counts.extend([world.received.len(), world.granted.len(), world.sockets]);
    }
    counts
}

/// Ticket 25's stop rule, for every follower and for the look: once
/// client hooks are off, nothing more is asked of any gateway.
#[test]
fn turning_client_hooks_off_ends_every_follower_and_the_look_at_the_machine() {
    let mut lab = lab();
    let (default, reports) = two(&mut lab, looking());
    let followers = lab.observer.as_ref().unwrap().followers();
    lab.observer.take();
    assert!(followers.iter().all(|follower| !follower.active()));
    assert!(lab.cards().is_empty());
    assert!(lab.requests.native_states().is_empty());
    // A step that began in the instant of the stop may still end; after
    // that, nothing is asked: no home, no ticket, no dial, no frame.
    std::thread::sleep(Duration::from_millis(150));
    let settled = asked(&lab, &[&default, &reports]);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(asked(&lab, &[&default, &reports]), settled);
    assert_eq!(lab.said("stopped").len(), 1, "{}", lab.log());
}

/// Ticket 13's rule, for the look: it waits on a gateway with nothing
/// locked. While a control socket holds its answer to the look, the set of
/// followers and every follower's model are free, every follower polls on,
/// and a hook's nudge returns at once.
#[test]
fn a_look_waits_on_a_gateway_with_nothing_locked() {
    let mut lab = lab();
    let (_default, _reports) = two(&mut lab, looking());
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("exchange", held.clone()));
    until("a look is waiting on a control socket", || {
        held.entered.load(Ordering::Acquire)
    });
    let observer = lab.observer.as_ref().unwrap();
    // A poll takes its follower's model for a moment now and then, so each
    // is looked at twenty times: one held across the look's wait would be
    // held every time.
    let followers = observer.followers();
    let mut free = vec![0; followers.len()];
    for _ in 0..20 {
        assert!(!observer.all.locked(), "the set is free");
        for (follower, free) in followers.iter().zip(&mut free) {
            *free += usize::from(follower.model.try_lock().is_ok());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(free.iter().all(|free| *free >= 10), "{free:?}");
    let asked = std::time::Instant::now();
    observer.nudge();
    assert!(asked.elapsed() < Duration::from_millis(250));
    for reader in ["local", "local/reports"] {
        let read = read_ms(&lab, reader).unwrap();
        until("the follower polls on while the look waits", || {
            read_ms(&lab, reader).is_some_and(|at| at > read)
        });
    }
    assert!(held.entered.load(Ordering::Acquire) && !held.released.load(Ordering::Acquire));
    // Turning hooks off does not wait for the look either.
    let stopping = std::time::Instant::now();
    lab.observer.take();
    assert!(stopping.elapsed() < Duration::from_millis(250));
    held.released.store(true, Ordering::Release);
}
