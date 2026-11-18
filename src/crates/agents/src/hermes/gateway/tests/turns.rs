//! Turns that start and end on a far machine (`turns.rs`, ticket 33), with
//! Hermes's hints withheld: nothing here nudges a poll. A turn is told
//! only where an event of it was read after a place this connection had
//! seen the stream at, and it is told once.
use super::super::*;
use super::fake::{ask, identified, names, poll, Hermes};
use super::{far, INSTALL};
use serde_json::json;

const S: &str = "far-session";
/// No turn told.
const NONE: [(String, u64, bool); 0] = [];

fn verify() -> Profile {
    Profile::named("verify").unwrap()
}

/// A far machine's gateway, ready, and a Hermes with nothing in it.
fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

fn starts(hermes: &mut Hermes, session: &str, turn: u64) {
    hermes.publish(&verify(), session, "message.start", turn, json!({}));
}

fn ends(hermes: &mut Hermes, session: &str, turn: u64) {
    let body = json!({"status": "complete", "outcome": "completed", "text": "Synthetic reply"});
    hermes.publish(&verify(), session, "message.complete", turn, body);
}

/// Events that are no turn's start or end.
fn works(hermes: &mut Hermes, session: &str, turn: u64, count: usize) {
    for _ in 0..count {
        let body = json!({"name": "terminal"});
        hermes.publish(&verify(), session, "tool.complete", turn, body);
    }
}

/// One whole poll, and the turns it told: each as the session's id, the
/// turn's count and whether it ended. A poll of these tests opens no card.
fn told(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) -> Vec<(String, u64, bool)> {
    let effects = poll(gateway, hermes, now_ms);
    assert!(
        effects.is_empty(),
        "no card is opened or closed here: {effects:?}"
    );
    said(&std::mem::take(&mut hermes.told))
}

/// The turns told since they were last taken, where no whole poll ran.
fn seen(gateway: &mut Gateway) -> Vec<(String, u64, bool)> {
    said(&gateway.take_turns())
}

fn said(turns: &[Turn]) -> Vec<(String, u64, bool)> {
    let said = |turn: &Turn| {
        // A turn says which, where and whether it ended. It carries no
        // time: nothing read says when a turn ended.
        let Turn {
            session,
            epoch: _,
            generation,
            ended,
        } = turn;
        (session.id().to_owned(), *generation, *ended)
    };
    turns.iter().map(said).collect()
}

fn started(turn: u64) -> (String, u64, bool) {
    (S.to_owned(), turn, false)
}

fn ended(turn: u64) -> (String, u64, bool) {
    (S.to_owned(), turn, true)
}

/// The looks and the reads for the turns that the last poll asked.
fn watched(hermes: &Hermes) -> Vec<Option<u64>> {
    let from = |step: &Step| match step {
        Step::Watch { cursor, .. } => Some(cursor.as_ref().map(|cursor| cursor.seq)),
        _ => None,
    };
    hermes.asked.iter().filter_map(from).collect()
}

#[test]
fn working_then_idle_without_a_card_ends_once() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    works(&mut h, S, 1, 3);
    // Listed working: the poll reads it, as it always did.
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    assert_eq!(g.statuses()[0].1, Status::Working);

    ends(&mut h, S, 1);
    // Listed idle, with no card: no poll read it before this ticket.
    assert_eq!(told(&mut g, &mut h, 2_000), [ended(1)]);
    assert_eq!(g.statuses()[0].1, Status::Idle);
    assert_eq!(watched(&h), [None, Some(4)]);
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);
}

#[test]
fn a_turn_wholly_between_two_polls_ends_once() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    // Seen before, and idle.
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    assert_eq!(g.statuses()[0].1, Status::Idle);

    starts(&mut h, S, 2);
    works(&mut h, S, 2, 2);
    ends(&mut h, S, 2);
    // Idle again by the next poll: the list never said it worked.
    assert_eq!(told(&mut g, &mut h, 11_000), [started(2), ended(2)]);
    assert_eq!(g.statuses()[0].1, Status::Idle);
    assert_eq!(told(&mut g, &mut h, 21_000), NONE);
}

#[test]
fn first_sight_tells_nothing() {
    let (mut g, mut h) = start();
    for turn in 1..=3 {
        starts(&mut h, S, turn);
        works(&mut h, S, turn, 2);
        ends(&mut h, S, turn);
    }
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    // One look, and nothing read: what is in the stream was there before.
    assert_eq!(watched(&h), [None]);
    assert_eq!(told(&mut g, &mut h, 2_000), NONE);

    // The same for a session first seen while it works: its turn's start
    // is in the stream the poll reads, and was there before.
    starts(&mut h, "far-other", 1);
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);
    assert_eq!(told(&mut g, &mut h, 4_000), NONE);
}

#[test]
fn a_session_gone_before_its_end_was_read_tells_nothing() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    assert_eq!(g.statuses()[0].1, Status::Working);

    // Its turn ends and it leaves the live list before the next poll.
    ends(&mut h, S, 1);
    let gone = h.streams.remove(&(verify(), S.to_owned())).unwrap();
    assert_eq!(told(&mut g, &mut h, 2_000), NONE);
    assert!(g.statuses().is_empty());

    // Listed again later, with the end still in its stream: it is seen
    // for the first time again, and what is there was there before.
    h.streams.insert((verify(), S.to_owned()), gone);
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);
    assert_eq!(told(&mut g, &mut h, 4_000), NONE);
}

#[test]
fn an_epoch_change_tells_nothing() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);

    // Hermes lost the stream's count and began another, in which a turn
    // ran: its events count from 1 again.
    h.new_epoch(&verify(), S, "epoch-2");
    starts(&mut h, S, 2);
    works(&mut h, S, 2, 6);
    ends(&mut h, S, 2);
    assert_eq!(told(&mut g, &mut h, 2_000), NONE);
    // Nothing was read from the old place in the new stream.
    assert_eq!(watched(&h), [None]);

    // The place starts again in the new epoch.
    starts(&mut h, S, 3);
    ends(&mut h, S, 3);
    assert!(poll(&mut g, &mut h, 3_000).is_empty());
    let turns = std::mem::take(&mut h.told);
    assert_eq!(turns.len(), 2);
    assert!(turns.iter().all(|turn| turn.epoch == "epoch-2"));
    assert_eq!(turns[1].id(), "epoch-2:3");
    assert_eq!(watched(&h), [None, Some(8)]);
}

#[test]
fn events_dropped_between_two_looks() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);

    // A turn runs and ends, and Hermes drops its first events: the place
    // the stream was seen at is among them, the end is not.
    starts(&mut h, S, 2);
    works(&mut h, S, 2, 40);
    ends(&mut h, S, 2);
    works(&mut h, S, 2, 5);
    h.drop_through(&verify(), S, 30);
    assert_eq!(told(&mut g, &mut h, 2_000), [ended(2)]);
    // The read from the old place was refused, and the end was found in
    // what Hermes still keeps.
    assert_eq!(watched(&h)[..2], [None, Some(2)]);
    assert!(watched(&h).len() > 2, "{:?}", watched(&h));
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);
}

#[test]
fn a_completion_that_was_itself_dropped_tells_nothing() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);

    // A turn ends, and so much follows that Hermes keeps its end no more.
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    works(&mut h, S, 2, 2_000);
    h.keep_what_hermes_keeps(&verify(), S);
    assert!(h.stream(&verify(), S).dropped > 4);
    // The stream moved 2,002 places past the one it was seen at. That a
    // stream moved is no evidence that a turn ended.
    assert_eq!(told(&mut g, &mut h, 2_000), NONE);
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);

    // What ends after that is told.
    starts(&mut h, S, 3);
    ends(&mut h, S, 3);
    assert_eq!(told(&mut g, &mut h, 4_000), [started(3), ended(3)]);
}

#[test]
fn a_session_whose_read_was_delayed() {
    let (mut g, mut h) = start();
    h.stream(&verify(), "far-other");
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    let at = g.session(&verify(), S).unwrap();
    let look = Step::Watch {
        session: at,
        cursor: None,
    };

    // The turn ends at once, and for five polls the look at this one
    // session fails. The socket stays open: the other session is looked
    // at, and the gateway stays ready.
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    for round in 1..=5 {
        h.failing.push(look.clone());
        assert_eq!(told(&mut g, &mut h, 1_000 + round * 10_000), NONE);
        assert!(h.failing.is_empty());
        assert_eq!(watched(&h).len(), 2);
        assert!(matches!(g.connection(), Connection::Ready(_)));
    }
    // It is told when it is read, fifty seconds after the look before
    // it, and once.
    assert_eq!(told(&mut g, &mut h, 61_000), [started(2), ended(2)]);
    assert_eq!(told(&mut g, &mut h, 71_000), NONE);

    // The same where the look is answered and the read that follows it
    // fails: the look saw the stream moved, which is no evidence of what
    // moved it, and the place stays until the events were read.
    starts(&mut h, S, 3);
    ends(&mut h, S, 3);
    let cursor = Some(Cursor {
        epoch: "epoch-1".into(),
        seq: 4,
    });
    let read = Step::Watch {
        session: g.session(&verify(), S).unwrap(),
        cursor,
    };
    for round in 1..=3 {
        h.failing.push(read.clone());
        assert_eq!(told(&mut g, &mut h, 71_000 + round * 10_000), NONE);
        assert!(h.failing.is_empty());
    }
    assert_eq!(told(&mut g, &mut h, 111_000), [started(3), ended(3)]);
    assert_eq!(told(&mut g, &mut h, 121_000), NONE);
}

#[test]
fn the_same_end_read_again_tells_nothing() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    assert_eq!(told(&mut g, &mut h, 2_000), [started(2), ended(2)]);
    for round in 1..=10 {
        assert_eq!(told(&mut g, &mut h, 2_000 + round * 1_000), NONE);
    }

    // A turn that runs on: every poll reads the whole stream again, the
    // end of the turn before it included.
    starts(&mut h, S, 3);
    assert_eq!(told(&mut g, &mut h, 20_000), [started(3)]);
    for round in 1..=10 {
        works(&mut h, S, 3, 1);
        assert_eq!(told(&mut g, &mut h, 20_000 + round * 1_000), NONE);
        assert_eq!(h.read_from(), [0], "read whole");
    }
}

#[test]
fn ten_idle_sessions_cost_one_call_each_in_each_poll() {
    let (mut g, mut h) = start();
    for index in 0..10 {
        let id = format!("far-idle-{index}");
        starts(&mut h, &id, 1);
        ends(&mut h, &id, 1);
    }
    for round in 0..3 {
        assert_eq!(told(&mut g, &mut h, 1_000 + round * 10_000), NONE);
        // The two lists the gateway's profiles always cost, and ten looks.
        assert_eq!(h.asked.len(), 12, "{:?}", h.asked);
        assert_eq!(watched(&h), [None; 10]);
    }

    // One of them moves: one read more, of that one.
    starts(&mut h, "far-idle-4", 2);
    ends(&mut h, "far-idle-4", 2);
    let turns = told(&mut g, &mut h, 40_000);
    assert_eq!(turns.len(), 2);
    assert!(turns
        .iter()
        .all(|(id, turn, _)| id == "far-idle-4" && *turn == 2));
    // And one call for what that turn used (`sample.rs`), in the round
    // of count calls after the poll.
    assert_eq!(h.asked.len(), 14);

    // A session the poll reads anyway costs no call more: its turns are
    // taken from what the poll read.
    starts(&mut h, "far-idle-4", 3);
    assert_eq!(told(&mut g, &mut h, 41_000).len(), 1);
    assert_eq!(watched(&h), [None; 9]);
    assert_eq!(h.read_from(), [0]);
    // And one call for what the session that works has used so far
    // (`sample.rs`), which is no look.
    assert_eq!(h.asked.len(), 2 + 9 + 2 + 1);
}

#[test]
fn this_machines_gateway_looks_at_nothing() {
    let mut g = Gateway::new(GatewayId::new(Machine::This, Profile::Default));
    identified(&mut g, "one");
    let mut h = Hermes::default();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    for round in 0..3 {
        starts(&mut h, S, 2 + round);
        ends(&mut h, S, 2 + round);
        assert_eq!(told(&mut g, &mut h, 1_000 * (round as i64 + 1)), NONE);
        // Its hooks tell its turns: the poll asks what it always asked.
        assert_eq!(
            h.asked,
            [Step::List(Profile::Default), Step::List(verify())]
        );
    }
}

#[test]
fn a_lost_socket_or_a_restart_forgets_every_place() {
    for restart in [false, true] {
        let (mut g, mut h) = start();
        starts(&mut h, S, 1);
        ends(&mut h, S, 1);
        assert_eq!(told(&mut g, &mut h, 1_000), NONE);

        // The link is cut, a turn ends meanwhile, and the link returns.
        g.apply(Input::Lost(Failure::Closed(1006)), 2_000);
        starts(&mut h, S, 2);
        ends(&mut h, S, 2);
        identified(&mut g, if restart { "two" } else { "one" });
        assert_eq!(told(&mut g, &mut h, 60_000), NONE, "restart {restart}");
        assert_eq!(watched(&h), [None]);
        // Seen again on this connection, the next end is told.
        starts(&mut h, S, 3);
        ends(&mut h, S, 3);
        assert_eq!(told(&mut g, &mut h, 61_000), [started(3), ended(3)]);
    }
}

#[test]
fn a_poll_that_is_dropped_sets_no_place_and_tells_nothing() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);

    // A poll looks at the stream, and comes back after the socket was
    // lost and found again. A turn ran while the socket was gone.
    let mut late = g.poll().unwrap();
    ask(&mut late, &mut h);
    g.apply(Input::Lost(Failure::Closed(1006)), 2_000);
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    identified(&mut g, "one");
    assert!(g.apply(Input::Polled(late), 3_000).is_empty());
    assert_eq!(seen(&mut g), NONE);
    // The place that poll saw is no place of this connection: the turn
    // that ran meanwhile is not told from it.
    assert_eq!(told(&mut g, &mut h, 4_000), NONE);
    // And a poll begun before a newer one, which read it too.
    assert_eq!(told(&mut g, &mut h, 4_000), NONE);
    starts(&mut h, S, 3);
    let mut older = g.poll().unwrap();
    ask(&mut older, &mut h);
    ends(&mut h, S, 3);
    assert_eq!(told(&mut g, &mut h, 5_000), [started(3), ended(3)]);
    assert!(g.apply(Input::Polled(older), 6_000).is_empty());
    assert_eq!(seen(&mut g), NONE);
}

#[test]
fn two_turns_between_two_polls_are_each_told_and_a_stopped_turn_ends_too() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    starts(&mut h, S, 3);
    // Stopped by its user: Hermes's own hook fires for it as for any end.
    let stopped = json!({"status": "interrupted", "outcome": "cancelled"});
    h.publish(&verify(), S, "message.complete", 3, stopped);
    assert_eq!(
        told(&mut g, &mut h, 11_000),
        [started(2), ended(2), started(3), ended(3)]
    );
}

#[test]
fn a_reading_of_one_session_apart_tells_its_turns_once() {
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    assert_eq!(told(&mut g, &mut h, 1_000), NONE);
    let at = g.session(&verify(), S).unwrap();
    ends(&mut h, S, 1);
    starts(&mut h, S, 2);

    // Read apart, as a session that is slow to read is: the reading
    // tells what it read past the place the stream was seen at.
    let mut apart = g.poll_apart(&at).expect("a reading apart");
    ask(&mut apart, &mut h);
    assert!(g.apply(Input::Polled(apart), 2_000).is_empty());
    assert_eq!(seen(&mut g), [ended(1), started(2)]);
    // The poll after it reads the same events and tells none again.
    assert_eq!(told(&mut g, &mut h, 3_000), NONE);
    ends(&mut h, S, 2);
    assert_eq!(told(&mut g, &mut h, 4_000), [ended(2)]);
}

/// The cards are what they were: with the looks on, a question of a far
/// session opens, and settles, exactly as before.
#[test]
fn a_card_opens_and_closes_as_it_did_beside_the_looks() {
    let (mut g, mut h) = start();
    h.stream(&verify(), "far-idle");
    starts(&mut h, S, 1);
    let body = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
    h.publish(&verify(), S, "clarify.request", 1, body);
    let opened = poll(&mut g, &mut h, 1_000);
    assert!(matches!(opened[..], [Effect::Open(_)]), "{opened:?}");
    assert_eq!(names(&g), ["p1"]);
    assert_eq!(said(&h.told), NONE);

    let settled = json!({"prompt_id": "p1", "execution_generation": 1});
    h.publish(&verify(), S, "clarify.settled", 1, settled);
    ends(&mut h, S, 1);
    let closed = poll(&mut g, &mut h, 2_000);
    assert!(matches!(closed[..], [Effect::Close { .. }]), "{closed:?}");
    assert_eq!(said(&h.told), [ended(1)]);
}

/// Every other test of the gateway model, the ticket 12 regressions and
/// the replays of the recordings among them, passes with the looks on
/// for every gateway and with them off for every gateway: the cards, and
/// the effects a poll gives, are the same either way. Each run is this
/// test program again, told which way by its environment.
#[test]
fn positions_decide_no_card() {
    let program = std::env::current_exe().unwrap();
    for looks in ["on", "off"] {
        let run = std::process::Command::new(&program)
            .env(super::super::turns::LOOKS, looks)
            .args(["hermes::gateway::tests::", "--test-threads", "2"])
            // These tests need the looks as they are, and one times a fold.
            .args(["--skip", "hermes::gateway::tests::turns"])
            .args(["--skip", "hermes::gateway::tests::cost"])
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&run.stdout);
        assert!(run.status.success(), "looks {looks}:\n{said}");
        // The regressions and the replays were among what ran.
        let passed = |name: &str| said.contains(&format!("tests::{name} ... ok"));
        assert!(
            passed("replay::every_recorded_scenario_gives_the_expected_cards_and_state"),
            "{said}"
        );
        // Counted by the lines that say a test passed: a slow machine
        // adds lines that name a test still running.
        let regression =
            |line: &&str| line.contains("tests::regressions::") && line.ends_with(" ... ok");
        let count = said.lines().filter(regression).count();
        assert_eq!(count, 23, "looks {looks}:\n{said}");
        assert!(said.contains(" 0 failed; 0 ignored"), "{said}");
    }
}

/// What a far follower tells core of a turn is the fact this machine's
/// hooks give of one (`hermes/hooks.rs`): the same kind, the same fields,
/// under the far session's id. It has one field more, the machine's
/// label, and its times are when it was seen.
#[test]
fn a_far_turn_is_told_as_the_hooks_tell_a_local_one() {
    const SEEN: i64 = 1_790_000_000_000;
    let (mut g, mut h) = start();
    starts(&mut h, S, 1);
    ends(&mut h, S, 1);
    poll(&mut g, &mut h, 1_000);
    starts(&mut h, S, 2);
    ends(&mut h, S, 2);
    poll(&mut g, &mut h, 2_000);
    let far: Vec<crate::HookEvent> = (h.told.iter())
        .map(|turn| turn.hook_event(SEEN, Some("Mac mini")))
        .collect();

    // The same two facts from this machine's hooks.
    let hook = |name: &str| {
        let payload = json!({"hook_event_name": name, "session_id": "20261104_101500_a1b2c3",
            "cwd": "/Users/USER/project", "extra": {"turn_id": "turn-2", "model": "m"}});
        let parsed = crate::parse_event("hermes", "--completion-hook", &payload, SEEN);
        parsed.unwrap().unwrap()
    };
    let near = [hook("pre_llm_call"), hook("on_session_end")];

    let fields = |event: &crate::HookEvent| {
        let mut keys: Vec<String> = event.data.as_object().unwrap().keys().cloned().collect();
        keys.retain(|key| key != "machine");
        keys
    };
    for (far, near) in far.iter().zip(&near) {
        assert_eq!(far.kind, near.kind);
        assert_eq!(far.source, near.source);
        assert_eq!(fields(far), fields(near), "{}", far.kind);
        assert_eq!(far.at_ms, SEEN);
        assert_eq!(far.data["machine"], "Mac mini");
        assert_eq!(far.data["turnId"], "epoch-1:2");
        let id = format!("hermes:{}/verify:{S}", far_key());
        assert_eq!(far.session_id, id);
    }
    assert_eq!(far[0].data["state"], "running");
    // When a turn was seen to have started is not when it started.
    assert!(far[0].data["startedAtMs"].is_null());
    assert_eq!(far[0].data["observedAtMs"], SEEN);
    // This machine's own turns carry no machine.
    let here = h.told[0].hook_event(SEEN, None);
    assert!(here.data.get("machine").is_none());
}

/// The key of the made-up far machine, as a session id writes it.
fn far_key() -> String {
    let Machine::Far(key) = far(INSTALL) else {
        unreachable!()
    };
    key.as_str().to_owned()
}

/// What a real gateway answered to these very reads (Hermes 0.22.5, the
/// throwaway gateway of lab-13; the fixture says how it was recorded): a
/// look returns the stream's epoch and latest sequence and no event, the
/// read from the place a look gave returns the turn that ran since, and
/// a `message.complete` carries no time at all.
#[test]
fn a_real_gateway_answers_the_looks_and_its_completion_carries_no_time() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hermes/gateway-turns/a-turn-read-after-a-look.json");
    let recorded: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let spike = Profile::named("hud-spike").unwrap();
    let mut g = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    let answer = super::fake::ready("one", &["default", "hud-spike"]);
    g.apply(
        Input::Identified {
            answer: &answer,
            home: "/synthetic",
        },
        0,
    );
    let id = recorded["listed_before"][0]["id"].as_str().unwrap();
    let at = g.session(&spike, id).unwrap();

    // One poll against what was recorded: each list, then each read for
    // the turns, answered as the real gateway answered it.
    let mut polled = |listed: &str, replies: &[&str], now_ms: i64| {
        let mut poll = g.poll().unwrap();
        let mut asked = Vec::new();
        for step in poll.steps() {
            let rows = match &step {
                Step::List(profile) if *profile == spike => recorded[listed].clone(),
                _ => json!([]),
            };
            poll.answer(&step, &json!({ "sessions": rows }));
        }
        for reply in replies {
            let steps = poll.steps();
            assert_eq!(steps.len(), 1, "{steps:?}");
            poll.answer(&steps[0], &recorded[*reply]["result"]);
            asked.push(steps[0].clone());
        }
        assert_eq!(poll.steps(), []);
        assert!(g.apply(Input::Polled(poll), now_ms).is_empty());
        (asked, g.take_turns())
    };

    // The look: no cursor, and Hermes answers with no event.
    let look = Step::Watch {
        session: at.clone(),
        cursor: None,
    };
    let (asked, turns) = polled("listed_before", &["look_before"], 1_000);
    assert_eq!(asked, std::slice::from_ref(&look));
    assert_eq!(turns, []);
    let before = &recorded["look_before"]["result"];
    assert_eq!(before["events"], json!([]));
    let epoch = before["replay_epoch"].as_str().unwrap();
    let place = before["last_sequence"].as_u64().unwrap();

    // A turn ran and ended, listed idle before and after. The next look
    // finds the stream moved, and the read from the place tells the turn.
    assert_eq!(recorded["listed_after"][0]["status"], "idle");
    let (asked, turns) = polled(
        "listed_after",
        &["look_after", "read_from_the_place"],
        2_000,
    );
    let cursor = Some(Cursor {
        epoch: epoch.to_owned(),
        seq: place,
    });
    let read = Step::Watch {
        session: at.clone(),
        cursor,
    };
    assert_eq!(asked, [look, read]);
    let told: Vec<(u64, bool)> = (turns.iter())
        .map(|turn| (turn.generation, turn.ended))
        .collect();
    assert_eq!(told, [(2, false), (2, true)]);
    assert_eq!(turns[1].id(), format!("{epoch}:2"));
    assert_eq!(turns[1].session, at);

    // Every field the completion carries. None of them is a time.
    let events = recorded["read_from_the_place"]["result"]["events"]
        .as_array()
        .unwrap();
    let complete = (events.iter())
        .find(|event| event["type"] == "message.complete")
        .unwrap();
    let keys = |of: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = of.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(
        keys(complete),
        [
            "admission_id",
            "authority_epoch",
            "execution_generation",
            "payload",
            "replay_epoch",
            "seq",
            "session_id",
            "type"
        ]
    );
    assert_eq!(
        keys(&complete["payload"]),
        [
            "admission_id",
            "content",
            "outcome",
            "persisted_turn",
            "status",
            "text",
            "usage"
        ]
    );
    assert_eq!(
        keys(&complete["payload"]["persisted_turn"]),
        [
            "complete",
            "final_assistant_row_id",
            "row_ids",
            "submission_id",
            "user_row_id",
            "user_row_ids"
        ]
    );
    // `avg_latency_s` is the mean latency of the turn's model calls.
    let timed: Vec<String> = (keys(&complete["payload"]["usage"]).into_iter())
        .filter(|key| key.contains("time") || key.ends_with("_at") || key.ends_with("_s"))
        .collect();
    assert_eq!(timed, ["avg_latency_s"]);
}

/// In a round, the reads the cards wait on are asked before the looks:
/// a gateway answers the calls of one socket in turn, so a card's read
/// never queues behind the looks at idle sessions.
#[test]
fn the_looks_go_out_after_the_reads_the_cards_wait_on() {
    let (mut g, mut h) = start();
    // Idle sessions that sort before and after the one that works.
    for id in ["far-a", "far-z"] {
        starts(&mut h, id, 1);
        ends(&mut h, id, 1);
    }
    starts(&mut h, "far-m", 1);
    let mut poll = g.poll().unwrap();
    for step in poll.steps() {
        assert!(matches!(step, Step::List(_)), "{step:?}");
        poll.answer(&step, &h.answer(&step).unwrap());
    }
    let asked: Vec<(&str, String)> = (poll.steps().iter())
        .map(|step| match step {
            Step::Since { session, .. } => ("read", session.id().to_owned()),
            Step::Watch { session, .. } => ("look", session.id().to_owned()),
            // What a session has used is asked in no poll (`sample.rs`).
            other => panic!("{other:?}"),
        })
        .collect();
    let expected = [("read", "far-m"), ("look", "far-a"), ("look", "far-z")];
    let expected: Vec<(&str, String)> = (expected.iter())
        .map(|(kind, id)| (*kind, (*id).to_owned()))
        .collect();
    assert_eq!(asked, expected);
}
