//! A session read apart from the polls (ticket 30, after its first
//! review): the polls go on without it and leave its cards alone, a
//! reading of it alone opens and closes them, and such a reading counts
//! for nothing once something happened that it could not know of.
use super::super::*;
use super::fake::{ask, identified, names, poll, Hermes};
use serde_json::json;

const SLOW: &str = "local-slow";
const OTHER: &str = "local-other";

fn work() -> Profile {
    Profile::named("verify").unwrap()
}

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(Machine::This, Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

fn slow(gateway: &Gateway) -> SessionRef {
    gateway.session(&work(), SLOW).unwrap()
}

fn question(id: &str) -> serde_json::Value {
    json!({"kind": "clarify", "prompt_id": id, "execution_generation": 1,
        "question": "Synthetic question", "choices": [], "timeout_s": 20})
}

fn asks(hermes: &mut Hermes, session: &str, id: &str) {
    if !hermes.stream(&work(), session).running {
        hermes.publish(&work(), session, "message.start", 1, json!({}));
    }
    hermes.publish(&work(), session, "clarify.request", 1, question(id));
}

fn settles(hermes: &mut Hermes, session: &str, id: &str) {
    let body = json!({"prompt_id": id, "execution_generation": 1});
    hermes.publish(&work(), session, "clarify.settled", 1, body);
}

/// One whole poll that does not read the slow session: each read of it
/// is said to have failed, as the connection says of a session it reads
/// apart.
fn poll_beside(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) -> Vec<Effect> {
    let mut poll = gateway.poll().expect("a poll");
    assert_eq!(poll.apart(), None);
    for _ in 0..64 {
        let steps = poll.steps();
        if steps.is_empty() {
            break;
        }
        for step in steps {
            let apart = match &step {
                Step::Since { session, .. } | Step::Snapshot(session) => session.id() == SLOW,
                Step::List(_) => false,
            };
            match hermes.answer(&step).filter(|_| !apart) {
                Some(result) => poll.answer(&step, &result),
                None => poll.failed(&step),
            }
        }
    }
    gateway.apply(Input::Polled(poll), now_ms)
}

/// One whole reading of the slow session apart, where one is begun.
fn read_apart(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) -> Option<Vec<Effect>> {
    let mut reading = gateway.poll_apart(&slow(gateway))?;
    assert_eq!(reading.apart(), Some(&slow(gateway)));
    ask(&mut reading, hermes);
    Some(gateway.apply(Input::Polled(reading), now_ms))
}

#[test]
fn the_polls_leave_a_session_read_apart_alone_and_its_own_reading_follows_it() {
    let (mut gateway, mut hermes) = start();
    asks(&mut hermes, SLOW, "slow-1");
    asks(&mut hermes, OTHER, "other-1");
    // The poll lists both and reads only the other: its card opens, and
    // nothing is known of the slow one.
    poll_beside(&mut gateway, &mut hermes, 1_000);
    assert_eq!(names(&gateway), ["other-1"]);
    let waits = Status::Waiting;
    assert_eq!(gateway.status(&slow(&gateway)), Some(waits));

    // Its own reading opens its card, and asks no list.
    let effects = read_apart(&mut gateway, &mut hermes, 1_100).unwrap();
    assert!(matches!(&effects[..], [Effect::Open(pending)] if pending.ask.key() == "slow-1"));
    assert!(hermes
        .asked
        .iter()
        .all(|step| !matches!(step, Step::List(_))));
    assert_eq!(names(&gateway), ["other-1", "slow-1"]);

    // Polls go on beside it: its card stays, the other's follows Hermes.
    settles(&mut hermes, OTHER, "other-1");
    asks(&mut hermes, SLOW, "slow-2");
    poll_beside(&mut gateway, &mut hermes, 2_000);
    assert_eq!(names(&gateway), ["slow-1"]);

    // The next reading of it brings its cards up to date, and touches no
    // other session's.
    asks(&mut hermes, OTHER, "other-2");
    settles(&mut hermes, SLOW, "slow-1");
    read_apart(&mut gateway, &mut hermes, 2_100).unwrap();
    assert_eq!(names(&gateway), ["slow-2"]);
    poll_beside(&mut gateway, &mut hermes, 3_000);
    assert_eq!(names(&gateway), ["other-2", "slow-2"]);
}

#[test]
fn a_reading_apart_is_begun_only_of_a_live_session_a_poll_would_read() {
    let (mut gateway, mut hermes) = start();
    let at = slow(&gateway);
    // Not listed by any poll yet.
    asks(&mut hermes, SLOW, "slow-1");
    assert!(gateway.poll_apart(&at).is_none());
    poll_beside(&mut gateway, &mut hermes, 1_000);
    assert!(gateway.poll_apart(&at).is_some());

    // Idle, with no card: a poll would not read it either.
    settles(&mut hermes, SLOW, "slow-1");
    hermes.publish(&work(), SLOW, "message.complete", 1, json!({}));
    poll_beside(&mut gateway, &mut hermes, 2_000);
    assert!(gateway.poll_apart(&at).is_none());

    // Another gateway's session, and a gateway that is not ready.
    let other = Gateway::new(GatewayId::new(Machine::This, work()));
    assert!(gateway
        .poll_apart(&other.session(&work(), SLOW).unwrap())
        .is_none());
    asks(&mut hermes, SLOW, "slow-2");
    poll_beside(&mut gateway, &mut hermes, 3_000);
    gateway.apply(Input::Lost(Failure::Closed(1006)), 3_100);
    assert!(gateway.poll_apart(&at).is_none());
}

/// Each of these happens while a reading apart is out. What it read
/// before then counts for nothing: applied, it changes no card.
#[test]
fn a_reading_begun_before_something_it_cannot_know_of_is_dropped() {
    type Meanwhile = fn(&mut Gateway, &mut Hermes);
    let answered: Meanwhile = |gateway, _| {
        // Answered here: Hermes took it, and the card closed.
        let id = gateway.waiting().next().unwrap().id();
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"status": "resolved"}});
        gateway.apply(
            Input::Responded {
                request: &id,
                reply: &reply,
            },
            1_200,
        );
    };
    let lost: Meanwhile = |gateway, _| {
        gateway.apply(Input::Lost(Failure::Closed(1006)), 1_200);
        identified(gateway, "one");
    };
    let restarted: Meanwhile = |gateway, _| {
        identified(gateway, "two");
    };
    let newer: Meanwhile = |gateway, hermes| {
        // A newer reading of the same session, which found it settled.
        settles(hermes, SLOW, "slow-1");
        read_apart(gateway, hermes, 1_200).unwrap();
    };
    let left: Meanwhile = |gateway, hermes| {
        // The session left the live list and came back in a new turn.
        hermes.streams.clear();
        poll_beside(gateway, hermes, 1_200);
        asks(hermes, SLOW, "slow-next");
        poll_beside(gateway, hermes, 1_300);
    };
    let retired: Meanwhile = |gateway, _| {
        let gone = [work()].into_iter().collect();
        gateway.apply(Input::Retired(&gone), 1_200);
    };
    // With what shows afterwards. A dropped socket closes nothing: the
    // card stays, marked, until a reading that counts finds it waiting.
    let cases: [(&str, Meanwhile, &[&str]); 6] = [
        ("answered", answered, &[]),
        ("lost", lost, &["slow-1"]),
        ("restarted", restarted, &[]),
        ("newer", newer, &[]),
        ("left", left, &[]),
        ("retired", retired, &[]),
    ];
    for (case, meanwhile, shows) in cases {
        let (mut gateway, mut hermes) = start();
        asks(&mut hermes, SLOW, "slow-1");
        poll_beside(&mut gateway, &mut hermes, 1_000);
        read_apart(&mut gateway, &mut hermes, 1_050).unwrap();
        assert_eq!(names(&gateway), ["slow-1"], "{case}");
        // A reading that still sees the request waiting, held back.
        let mut late = gateway.poll_apart(&slow(&gateway)).unwrap();
        ask(&mut late, &mut hermes);
        meanwhile(&mut gateway, &mut hermes);
        assert_eq!(names(&gateway), shows, "{case}");
        let marked = |gateway: &Gateway| {
            let ids: Vec<String> = gateway.waiting().map(|pending| pending.id()).collect();
            ids.iter().filter(|id| gateway.marked(id)).count()
        };
        let before = marked(&gateway);
        assert_eq!(before, usize::from(case == "lost"), "{case}");
        let effects = gateway.apply(Input::Polled(late), 1_400);
        assert_eq!(effects, [], "{case}");
        assert_eq!(names(&gateway), shows, "{case}");
        assert_eq!(
            marked(&gateway),
            before,
            "{case}: a stale reading took the mark off"
        );
    }
}

#[test]
fn a_reading_apart_takes_the_snapshot_the_events_call_for_and_not_another_too_soon() {
    let (mut gateway, mut hermes) = start();
    // A request that waits and that no event tells of.
    hermes.publish(&work(), SLOW, "message.start", 1, json!({}));
    hermes
        .stream(&work(), SLOW)
        .prompts
        .push(question("hidden-1"));
    poll_beside(&mut gateway, &mut hermes, 1_000);
    assert_eq!(names(&gateway), Vec::<String>::new());
    read_apart(&mut gateway, &mut hermes, 1_100).unwrap();
    assert_eq!(hermes.snapshots(), 1);
    assert_eq!(names(&gateway), ["hidden-1"]);

    // A second one, no event of it either: found by the next snapshot,
    // which is not taken before the one before is 30 s old.
    hermes
        .stream(&work(), SLOW)
        .prompts
        .push(question("hidden-2"));
    poll_beside(&mut gateway, &mut hermes, 2_000);
    read_apart(&mut gateway, &mut hermes, 2_100).unwrap();
    assert_eq!(hermes.snapshots(), 0, "too soon for another");
    assert_eq!(names(&gateway), ["hidden-1"]);
    let later = 1_100 + SNAPSHOT_EVERY_MS;
    poll_beside(&mut gateway, &mut hermes, later);
    read_apart(&mut gateway, &mut hermes, later + 100).unwrap();
    assert_eq!(hermes.snapshots(), 1);
    assert_eq!(names(&gateway), ["hidden-1", "hidden-2"]);
}

#[test]
fn a_reading_apart_tells_nothing_of_any_other_session() {
    let (mut gateway, mut hermes) = start();
    asks(&mut hermes, SLOW, "slow-1");
    asks(&mut hermes, OTHER, "other-1");
    poll(&mut gateway, &mut hermes, 1_000);
    assert_eq!(names(&gateway), ["other-1", "slow-1"]);
    let (statuses, states) = (gateway.statuses(), gateway.live_states());
    // The other session's request settles: only a poll may say so.
    settles(&mut hermes, OTHER, "other-1");
    hermes.publish(&work(), OTHER, "message.complete", 1, json!({}));
    let effects = read_apart(&mut gateway, &mut hermes, 5_000).unwrap();
    assert_eq!(effects, []);
    assert_eq!(names(&gateway), ["other-1", "slow-1"]);
    assert_eq!(gateway.statuses(), statuses);
    assert_eq!(
        gateway.live_states(),
        states,
        "no reading of the list was renewed"
    );
    // A profile two gateways claim gets no new card from it either.
    asks(&mut hermes, SLOW, "slow-2");
    let contested = [work()].into_iter().collect();
    gateway.apply(Input::Withheld(&contested), 5_100);
    read_apart(&mut gateway, &mut hermes, 5_200).unwrap();
    assert_eq!(names(&gateway), ["other-1", "slow-1"]);
}
