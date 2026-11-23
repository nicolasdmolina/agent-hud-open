//! The order a far gateway's poll asks in (`order.rs`, ticket 72): what
//! nothing has changed in (the reads of quiet sessions, the looks at
//! idle ones) is asked once every other read is answered, so that the
//! card of a session that just asked never queues behind a call for a
//! session that stands still.
use super::super::*;
use super::fake::{identified, names, poll, Hermes};
use super::{far, local, INSTALL};
use serde_json::json;

const STUCK: &str = "stuck";

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

fn listed_as_working(hermes: &mut Hermes, session: &str) {
    hermes.stream(&Profile::Default, session).status = Some("working".into());
}

/// A question asked in `session`: Hermes lists it as waiting from now.
fn asks(hermes: &mut Hermes, session: &str, prompt: &str) {
    let question = json!({"kind": "clarify", "prompt_id": prompt, "execution_generation": 1,
        "question": "Synthetic question", "choices": [], "timeout_s": 20});
    hermes.publish(&Profile::Default, session, "message.start", 1, json!({}));
    hermes.publish(&Profile::Default, session, "clarify.request", 1, question);
}

/// One whole poll, round by round: what each round asked.
fn rounds(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) -> Vec<Vec<Step>> {
    let mut poll = gateway.poll().expect("a gateway that is ready");
    let mut rounds = Vec::new();
    for _ in 0..64 {
        let steps = poll.steps();
        if steps.is_empty() {
            break;
        }
        for step in &steps {
            match hermes.answer(step) {
                Some(result) => poll.answer(step, &result),
                None => poll.failed(step),
            }
        }
        rounds.push(steps);
    }
    assert!(poll.steps().is_empty(), "the poll never ends");
    gateway.apply(Input::Polled(poll), now_ms);
    rounds
}

/// The rounds that read `session`'s events, by their place in the poll.
fn read_in(rounds: &[Vec<Step>], session: &str) -> Vec<usize> {
    let reads = |step: &Step| matches!(step, Step::Since { session: at, .. } if at.id() == session);
    let places = rounds.iter().enumerate();
    let read = places.filter(|(_, round)| round.iter().any(reads));
    read.map(|(place, _)| place).collect()
}

/// The rounds that look at an idle session's stream, or read on from one.
fn looked_in(rounds: &[Vec<Step>]) -> Vec<usize> {
    let looks = |step: &Step| matches!(step, Step::Watch { .. });
    let places = rounds.iter().enumerate();
    let looked = places.filter(|(_, round)| round.iter().any(looks));
    looked.map(|(place, _)| place).collect()
}

#[test]
fn on_a_far_gateway_what_nothing_changed_in_is_asked_after_every_other_read() {
    // The session that asks sorts before the quiet one, or after it:
    // the order is no accident of the names.
    for asker in ["old-a", "zed"] {
        let (mut g, mut h) = start();
        for id in ["old-a", "old-b", "zed"] {
            h.stream(&Profile::Default, id);
        }
        listed_as_working(&mut h, STUCK);
        poll(&mut g, &mut h, 0);
        poll(&mut g, &mut h, QUIET_MS);
        assert!(!g.lively(), "nothing moved in any of them");

        asks(&mut h, asker, "p1");
        let rounds = rounds(&mut g, &mut h, QUIET_MS + IDLE_POLL_MS);
        assert_eq!(names(&g), ["p1"], "{asker}: its card opens in that poll");
        let (asked, still) = (read_in(&rounds, asker), read_in(&rounds, STUCK));
        assert!(asked.len() >= 2, "{asker}: read to its end, {rounds:?}");
        // The quiet session is read as it was before, and after the one
        // that asked is read whole.
        assert!(!still.is_empty(), "{asker}: a quiet session is still read");
        let done = asked.iter().max().unwrap();
        assert!(
            still.iter().all(|place| place > done),
            "{asker}: the quiet session's read in rounds {still:?}, the asker's in {asked:?}"
        );
        // So are the looks at the sessions that stayed idle.
        let looked = looked_in(&rounds);
        assert!(
            looked.iter().all(|place| place > done),
            "{asker}: looks in rounds {looked:?}, the asker's reads in {asked:?}"
        );
    }
}

#[test]
fn a_quiet_session_is_read_with_the_first_once_the_list_says_something_else_or_its_card_shows() {
    let (mut g, mut h) = start();
    // Two sessions listed as working for days; one of them asks.
    listed_as_working(&mut h, STUCK);
    listed_as_working(&mut h, "stuck-too");
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, QUIET_MS);
    assert!(!g.lively());
    h.stream(&Profile::Default, STUCK).status = None;
    asks(&mut h, STUCK, "p1");
    let first = rounds(&mut g, &mut h, QUIET_MS + IDLE_POLL_MS);
    assert_eq!(names(&g), ["p1"]);
    let (asked, still) = (read_in(&first, STUCK), read_in(&first, "stuck-too"));
    let done = asked.iter().max().expect("the session that asked is read");
    assert!(
        still.iter().all(|place| place > done) && !still.is_empty(),
        "listed as waiting where it worked: {asked:?} before {still:?}"
    );
    // Its card shows for longer than the quiet time, and nothing more
    // happens in it. A card rests on its read: still among the first,
    // also in a poll that begins with the session known to be quiet.
    rounds(&mut g, &mut h, 3 * QUIET_MS);
    let later = rounds(&mut g, &mut h, 3 * QUIET_MS + 1_000);
    assert_eq!(names(&g), ["p1"]);
    let (shown, still) = (read_in(&later, STUCK), read_in(&later, "stuck-too"));
    let done = shown.iter().max().expect("a session with a card is read");
    assert!(
        still.iter().all(|place| place > done) && !still.is_empty(),
        "a card shows in it: {shown:?} before {still:?}"
    );
}

#[test]
fn a_gateway_of_this_machine_asks_in_the_order_it_did() {
    let mut g = local();
    identified(&mut g, "one");
    let mut h = Hermes::default();
    h.stream(&Profile::Default, "zed");
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(g.poll_ms(), IDLE_POLL_MS, "quiet, as the model counts it");
    asks(&mut h, "zed", "p1");
    let rounds = rounds(&mut g, &mut h, QUIET_MS + 800);
    assert_eq!(names(&g), ["p1"]);
    // Both are read from the round after the lists, side by side.
    let (asked, still) = (read_in(&rounds, "zed"), read_in(&rounds, STUCK));
    assert_eq!(asked.first(), still.first(), "{rounds:?}");
    assert_eq!(asked.first(), Some(&1), "{rounds:?}");
}

#[test]
fn a_poll_says_when_all_it_asks_is_for_sessions_nothing_changed_in() {
    let (mut g, mut h) = start();
    h.stream(&Profile::Default, "old-a");
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, QUIET_MS);
    asks(&mut h, "old-a", "p1");
    let mut asking = g.poll().unwrap();
    let mut said = Vec::new();
    while !asking.steps().is_empty() {
        let steps = asking.steps();
        let quiet = |step: &Step| match step {
            Step::Since { session, .. } => session.id() == STUCK,
            Step::Watch { .. } => true,
            _ => false,
        };
        // Last, and only then, where every step is for what stood still.
        assert_eq!(asking.last(), steps.iter().all(quiet), "{steps:?}");
        said.push(asking.last());
        for step in steps {
            asking.answer(&step, &h.answer(&step).unwrap());
        }
    }
    // The lists and the asker's two reads; then the quiet session's.
    assert_eq!(said, [false, false, false, true, true]);
    // A poll with nothing quiet in it reaches its looks last, if it
    // looks at all, and a gateway of this machine asks nothing last
    // before its poll is over.
    let mut here = local();
    identified(&mut here, "one");
    listed_as_working(&mut h, "old-a");
    poll(&mut here, &mut h, 0);
    poll(&mut here, &mut h, QUIET_MS);
    let mut asking = here.poll().unwrap();
    while !asking.steps().is_empty() {
        assert!(!asking.last(), "{:?}", asking.steps());
        for step in asking.steps() {
            asking.answer(&step, &h.answer(&step).unwrap());
        }
    }
}
