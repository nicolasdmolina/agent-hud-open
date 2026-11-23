//! The looks a far gateway's polls take at its idle sessions, for their
//! turns (ticket 33): what they ask of the gateway, and that a look
//! which is slow holds the polls up once and no more.
use super::{far_lab, pace, until, FarLab};
use crate::clients::hermes::{
    tests_unix::fake::{Fake, Hold},
    Pace,
};
use serde_json::Value;
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

const PROFILE: &str = "default";

fn following(pace: Pace) -> Option<(FarLab, Fake)> {
    let mut far = far_lab()?;
    let gateway = far.gateway("instance-1");
    far.follow_at(pace);
    far.followed();
    until("the polls run", || !gateway.world().listed.is_empty());
    Some((far, gateway))
}

/// A session that ran a turn before it was ever seen: listed, idle.
fn idle(gateway: &Fake, session: &str) {
    gateway.tells_of_turn(PROFILE, session, "message.start", 1);
    gateway.tells_of_turn(PROFILE, session, "message.complete", 1);
}

/// Every read of a stream the gateway was asked for, as the session and
/// where it was read from: nowhere for a look.
fn reads(gateway: &Fake) -> Vec<(String, Option<u64>)> {
    let calls = gateway.world().calls("session.events.since");
    let read = |(_, frame): &(usize, Value)| {
        let params = &frame["params"];
        let from = params
            .get("replay_epoch")
            .map(|_| params["last_sequence"].as_u64().unwrap());
        (params["session_id"].as_str().unwrap().to_owned(), from)
    };
    calls.iter().map(read).collect()
}

/// How many polls the gateway has seen: each lists the one profile once.
fn polls(gateway: &Fake) -> usize {
    let calls = gateway.world().calls("session.active_list");
    let its = |(_, frame): &&(usize, Value)| frame["params"]["profile"] == PROFILE;
    calls.iter().filter(its).count()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_idle_session_costs_one_look_a_poll_and_a_read_when_it_moved() {
    let Some((far, gateway)) = following(pace()) else {
        return;
    };
    for session in ["s-one", "s-two", "s-three"] {
        idle(&gateway, session);
    }
    until("the three are looked at", || reads(&gateway).len() >= 3);
    // Twenty polls, counted between two lists of the one profile: each
    // looks once at each of the three, and reads nothing.
    let (from, asked) = (polls(&gateway), reads(&gateway).len());
    until("twenty polls more", || polls(&gateway) >= from + 20);
    let (polled, looked) = (polls(&gateway) - from, reads(&gateway).len() - asked);
    let each = looked as f64 / polled as f64;
    assert!(
        (2.5..=3.5).contains(&each),
        "{looked} looks in {polled} polls"
    );
    assert!(reads(&gateway).iter().all(|(_, from)| from.is_none()));

    // One of them runs a turn between two polls: it is read on from the
    // place it was seen at, once, and the turn is told.
    gateway.tells_of_turn(PROFILE, "s-two", "message.start", 2);
    gateway.tells_of_turn(PROFILE, "s-two", "message.complete", 2);
    until("the turn is told", || {
        far.log().contains("turn ended session=")
    });
    let read: Vec<_> = (reads(&gateway).into_iter())
        .filter(|(_, from)| from.is_some())
        .collect();
    assert_eq!(read, [("s-two".to_owned(), Some(2))]);
    let log = far.log();
    assert_eq!(log.matches("turn started session=").count(), 1, "{log}");
    assert_eq!(log.matches("turn ended session=").count(), 1, "{log}");
    assert!(log.contains("turn=epoch-1:2"), "{log}");
    // No look ever made a session one that is read apart.
    assert!(!log.contains("apart session="), "{log}");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_look_that_is_slow_rests_its_session_and_tells_its_turn_later() {
    // A read is slow from 100 ms on, and a session rests six times as
    // long as a call may take: three seconds here.
    let pace = Pace {
        apart: Duration::from_millis(100),
        answer: Duration::from_millis(500),
        trace: true,
        ..pace()
    };
    let Some((far, gateway)) = following(pace) else {
        return;
    };
    idle(&gateway, "s-quick");
    idle(&gateway, "s-slow");
    until("both are looked at", || reads(&gateway).len() >= 4);

    // From now each read of one session's events takes 300 ms, and a
    // turn ran in it.
    gateway.tells_of_turn(PROFILE, "s-slow", "message.start", 2);
    gateway.tells_of_turn(PROFILE, "s-slow", "message.complete", 2);
    gateway.world().slow_replay = Some(("s-slow".into(), Duration::from_millis(300)));
    let of = |gateway: &Fake, session: &str| {
        let its = |(id, _): &(String, Option<u64>)| id == session;
        reads(gateway).into_iter().filter(its).count()
    };
    // The read that follows its look is slow, and is waited for: it is
    // what tells the turn. A rest holds back looks, never that read.
    until("the turn is told", || {
        far.log().contains("turn ended session=")
    });
    let (slow, quick, from) = (
        of(&gateway, "s-slow"),
        of(&gateway, "s-quick"),
        polls(&gateway),
    );
    // Then the session rests: thirty polls go by and it is asked nothing,
    // while the other is looked at in each. No lane was made for it.
    until("thirty polls more", || polls(&gateway) >= from + 30);
    assert_eq!(of(&gateway, "s-slow"), slow, "it rests");
    assert!(of(&gateway, "s-quick") >= quick + 25);
    assert!(!far.log().contains("apart session="), "{}", far.log());

    // A turn that runs while it rests is told when it is looked at again.
    gateway.tells_of_turn(PROFILE, "s-slow", "message.start", 3);
    gateway.tells_of_turn(PROFILE, "s-slow", "message.complete", 3);
    until("the later turn is told", || {
        far.log().matches("turn ended session=").count() == 2
    });
    assert!(far.log().contains("turn=epoch-1:3"), "{}", far.log());
}

/// A look that is never answered holds its poll up for as long as a call
/// may take, once. Its session then rests, so the polls after it are not
/// each held up in turn, and every card with them.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_look_that_is_never_answered_rests_its_session_and_the_polls_go_on() {
    // A call may take a second, so a session rests six.
    let pace = Pace {
        answer: Duration::from_secs(1),
        ..pace()
    };
    let Some((far, gateway)) = following(pace) else {
        return;
    };
    idle(&gateway, "s-quick");
    idle(&gateway, "s-mute");
    until("both are looked at", || reads(&gateway).len() >= 4);
    let of = |gateway: &Fake, session: &str| {
        let its = |(id, _): &(String, Option<u64>)| id == session;
        reads(gateway).into_iter().filter(its).count()
    };

    // From now the gateway says nothing to a look at one of them, and a
    // turn runs in it.
    let asked = of(&gateway, "s-mute");
    gateway.world().mute = Some("s-mute".into());
    gateway.tells_of_turn(PROFILE, "s-mute", "message.start", 2);
    gateway.tells_of_turn(PROFILE, "s-mute", "message.complete", 2);
    until("it is looked at, and not answered", || {
        of(&gateway, "s-mute") > asked
    });
    // Ten more polls look at the other session. None of them looks at
    // the one that did not answer: it rests.
    let (unanswered, quick) = (of(&gateway, "s-mute"), of(&gateway, "s-quick"));
    until("ten polls more", || of(&gateway, "s-quick") >= quick + 10);
    assert_eq!(of(&gateway, "s-mute"), unanswered, "it rests");
    assert!(!far.log().contains("turn ended session="), "{}", far.log());
    assert!(!far.log().contains("apart session="), "{}", far.log());

    // It answers again: the turn that ran meanwhile is told once the
    // rest is over, from the place the stream was last seen at.
    gateway.world().mute = None;
    until("the turn is told", || {
        far.log().contains("turn ended session=")
    });
    assert!(far.log().contains("turn=epoch-1:2"), "{}", far.log());
}

/// A look that only waited behind a slow one does not rest (ticket 73).
/// A gateway answers the calls of one socket in turn, so when the time
/// for what a poll asks last is up, every look sent after the slow one
/// is unanswered too. Only the one the gateway was at was slow. The
/// others are looked at again at the next poll, and a turn that ended
/// in one of them is told then, not a rest later.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_look_that_only_waited_behind_a_slow_one_does_not_rest() {
    // What is asked last has half a second. A call may take two, so a
    // session rests twelve.
    let pace = Pace {
        answer: Duration::from_secs(2),
        still: Duration::from_millis(500),
        trace: true,
        ..pace()
    };
    let Some((far, gateway)) = following(pace) else {
        return;
    };
    // The one that is held sorts first: its look is asked first.
    idle(&gateway, "s-a-held");
    idle(&gateway, "s-b-behind");
    until("both are looked at", || reads(&gateway).len() >= 4);
    let of = |gateway: &Fake, session: &str| {
        let its = |(id, _): &(String, Option<u64>)| id == session;
        reads(gateway).into_iter().filter(its).count()
    };
    let ended = |far: &FarLab| far.log().matches(" trace poll ms=").count();

    // The gateway holds its answer to the next look at the first, and
    // with it the answer to the look at the second, sent in the same
    // round. A turn runs in the second meanwhile.
    let held = Arc::new(Hold::default());
    {
        let mut world = gateway.world();
        world.slow_replay = Some(("s-a-held".into(), Duration::ZERO));
        world.hold_next = Some(("slow-probe", held.clone()));
    }
    until("the look is held", || held.entered.load(Ordering::Acquire));
    let polled = ended(&far);
    gateway.tells_of_turn(PROFILE, "s-b-behind", "message.start", 2);
    gateway.tells_of_turn(PROFILE, "s-b-behind", "message.complete", 2);
    until("the poll gives the looks up", || ended(&far) > polled);
    let looked = of(&gateway, "s-a-held");
    held.released.store(true, Ordering::Release);
    let released = Instant::now();

    until("the turn is told", || {
        far.log().contains("turn ended session=")
    });
    let told = released.elapsed();
    println!(
        "a turn behind a look that was held: told {} ms after",
        told.as_millis()
    );
    assert!(
        told < Duration::from_secs(6),
        "told {} ms after: its session rested",
        told.as_millis()
    );
    assert!(far.log().contains("turn=epoch-1:2"), "{}", far.log());
    // The one the gateway was at does rest.
    assert_eq!(of(&gateway, "s-a-held"), looked, "it rests");
}
