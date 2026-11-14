//! Cards whose gateway is out of reach (`reach.rs`): marked when the
//! socket is lost or an answer could not be sent, unmarked by the poll
//! that finds the request waiting again, and never closed for it.
use super::super::*;
use super::fake::{identified, names, poll, Hermes};
use serde_json::{json, Value};

const S: &str = "local-aaaa";

fn work() -> Profile {
    Profile::named("verify").unwrap()
}

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(Machine::This, Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

fn asks(hermes: &mut Hermes, session: &str, prompt: &str) {
    let body: Value = json!({"kind": "clarify", "prompt_id": prompt, "execution_generation": 1,
        "question": "Which one?", "choices": ["a", "b"], "multi_select": false});
    hermes.publish(&work(), session, "clarify.request", 1, body);
}

fn settles(hermes: &mut Hermes, session: &str, prompt: &str) {
    let body = json!({"prompt_id": prompt, "execution_generation": 1});
    hermes.publish(&work(), session, "clarify.settled", 1, body);
}

fn id(gateway: &Gateway, session: &str, prompt: &str) -> String {
    gateway
        .session(&work(), session)
        .unwrap()
        .request_id(prompt)
}

fn marks(effects: &[Effect]) -> Vec<String> {
    let said = |effect: &Effect| match effect {
        Effect::Marked(pending) => Some(format!("marked {}", pending.ask.key())),
        Effect::Unmarked(pending) => Some(format!("unmarked {}", pending.ask.key())),
        Effect::Close { id, why, .. } => {
            Some(format!("closed {} {why:?}", id.rsplit(':').next().unwrap()))
        }
        Effect::Open(pending) => Some(format!("opened {}", pending.ask.key())),
        _ => None,
    };
    effects.iter().filter_map(said).collect()
}

#[test]
fn a_lost_socket_marks_every_card_and_the_poll_that_finds_them_waiting_unmarks_them() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    asks(&mut h, "local-bbbb", "p2");
    poll(&mut g, &mut h, 0);
    let (one, two) = (id(&g, S, "p1"), id(&g, "local-bbbb", "p2"));
    assert!(!g.marked(&one) && !g.marked(&two));

    let lost = g.apply(Input::Lost(Failure::Closed(1006)), 10);
    assert_eq!(marks(&lost), ["marked p1", "marked p2"]);
    assert!(g.marked(&one) && g.marked(&two));
    // Still marked while it is dialled and while it is ready and unread.
    assert_eq!(marks(&g.apply(Input::Dialing, 20)), [] as [&str; 0]);
    assert_eq!(marks(&identified(&mut g, "one")), [] as [&str; 0]);
    assert!(g.marked(&one) && g.marked(&two));

    // The first poll that counts finds both waiting.
    assert_eq!(
        marks(&poll(&mut g, &mut h, 30)),
        ["unmarked p1", "unmarked p2"]
    );
    assert!(!g.marked(&one) && !g.marked(&two));
    assert_eq!(marks(&poll(&mut g, &mut h, 40)), [] as [&str; 0]);
    assert_eq!(names(&g), ["p1", "p2"]);
}

/// However often the gateway cannot be reached, a card is marked once and
/// nothing else is said of it: no close, no second mark.
#[test]
fn five_failed_reconnects_say_nothing_more_of_a_marked_card() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    poll(&mut g, &mut h, 0);
    let request = id(&g, S, "p1");
    assert_eq!(
        marks(&g.apply(Input::Lost(Failure::Closed(1006)), 1)),
        ["marked p1"]
    );
    for attempt in 0..5 {
        let now = 10 + attempt;
        assert_eq!(marks(&g.apply(Input::Dialing, now)), [] as [&str; 0]);
        let again = g.apply(Input::Lost(Failure::Unreachable), now);
        assert_eq!(marks(&again), [] as [&str; 0], "attempt {attempt}");
        assert!(g.poll().is_none());
    }
    assert_eq!(names(&g), ["p1"]);
    assert!(g.marked(&request));
    assert_eq!(g.pending(&request).unwrap().wait_ms, None);
}

/// What ended during the outage closes at the first poll as Hermes says,
/// and is never unmarked first; what still waits beside it is unmarked.
#[test]
fn a_request_that_is_over_closes_at_the_first_poll_and_one_that_waits_is_unmarked() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    asks(&mut h, S, "p2");
    poll(&mut g, &mut h, 0);
    g.apply(Input::Lost(Failure::Closed(1006)), 1);
    settles(&mut h, S, "p1");
    identified(&mut g, "one");
    assert_eq!(
        marks(&poll(&mut g, &mut h, 2)),
        ["closed p1 Settled", "unmarked p2"]
    );
    assert_eq!(names(&g), ["p2"]);
    assert!(!g.marked(&id(&g, S, "p1")));
}

/// A poll that could not tell whether the request waits (its profile's
/// list failed, or its session's read did) leaves the mark where it is.
#[test]
fn a_poll_that_could_not_tell_leaves_the_mark() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    poll(&mut g, &mut h, 0);
    let request = id(&g, S, "p1");
    let at = g.session(&work(), S).unwrap();
    g.apply(Input::Lost(Failure::Closed(1006)), 1);
    identified(&mut g, "one");
    for failing in [
        Step::List(work()),
        Step::Since {
            session: at.clone(),
            cursor: None,
        },
    ] {
        h.failing.push(failing);
        assert_eq!(marks(&poll(&mut g, &mut h, 2)), [] as [&str; 0]);
        assert!(g.marked(&request));
    }
    assert_eq!(marks(&poll(&mut g, &mut h, 3)), ["unmarked p1"]);
    assert!(!g.marked(&request));
    // And a poll that could not tell marks nothing that was not marked.
    h.failing.push(Step::List(work()));
    assert_eq!(marks(&poll(&mut g, &mut h, 4)), [] as [&str; 0]);
    assert!(!g.marked(&request));
}

/// An answer that could not be sent marks its card alone, asks for a poll
/// at once, and the poll that finds the request waiting unmarks it.
#[test]
fn an_answer_that_could_not_be_sent_marks_its_card_until_a_poll_finds_it_waiting() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    asks(&mut h, S, "p2");
    poll(&mut g, &mut h, 0);
    let (one, two) = (id(&g, S, "p1"), id(&g, S, "p2"));
    let effects = g.apply(Input::Unsent { request: &one }, 1);
    assert_eq!(marks(&effects), ["marked p1"]);
    assert!(effects.contains(&Effect::Poll));
    assert!(g.marked(&one) && !g.marked(&two));
    // Said once, however often it fails.
    let again = g.apply(Input::Unsent { request: &one }, 2);
    assert_eq!(marks(&again), [] as [&str; 0]);
    assert_eq!(marks(&poll(&mut g, &mut h, 3)), ["unmarked p1"]);
    // Nothing is said of a request that has no card.
    assert_eq!(g.apply(Input::Unsent { request: "none" }, 4), []);
}

/// A restart still closes a marked card: being out of reach never did.
#[test]
fn a_new_instance_closes_a_marked_card_as_a_restart() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    poll(&mut g, &mut h, 0);
    g.apply(Input::Lost(Failure::Closed(1006)), 1);
    assert_eq!(
        marks(&identified(&mut g, "two")),
        ["closed p1 GatewayRestarted"]
    );
    assert!(names(&g).is_empty());
}

/// The mark is one field on the card, in the adapter's words; a card that
/// can be answered does not have the field at all.
#[test]
fn a_marked_card_is_the_same_card_with_the_reason_on_it() {
    let (mut g, mut h) = start();
    asks(&mut h, S, "p1");
    poll(&mut g, &mut h, 0);
    let pending = g.waiting().next().unwrap().clone();
    let plain = pending.view(1_000, None);
    assert!(plain.get("unreachable").is_none());
    let mut marked = pending.view_out_of_reach(1_000, None);
    assert_eq!(marked["unreachable"], json!(out_of_reach()));
    assert!(!out_of_reach().is_empty());
    marked.as_object_mut().unwrap().remove("unreachable");
    assert_eq!(marked, plain);
    // On a far machine it keeps its machine's label.
    let far = pending.view_out_of_reach(1_000, Some("Mac mini"));
    assert_eq!(far["machine"], "Mac mini");
}
