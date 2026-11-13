//! What each live session is told to the HUD as doing (`live.rs`): what
//! the newest poll listed, waiting where a card shows, and nothing that
//! has not been read since the gateway was last ready.
use super::super::*;
use super::fake::{identified, poll, Hermes};
use crate::LiveState::{self, Idle, Running, Waiting};
use serde_json::json;

fn verify() -> Profile {
    Profile::named("verify")
}

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new("local"));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

fn lists(hermes: &mut Hermes, profile: &Profile, session: &str, status: &str) {
    hermes.stream(profile, session).status = Some(status.into());
}

fn told(gateway: &Gateway) -> Vec<(String, LiveState)> {
    gateway.live_states().sessions
}

#[test]
fn every_live_session_is_told_as_the_gateway_lists_it() {
    let (mut g, mut h) = start();
    lists(&mut h, &Profile::Default, "local-cron", "working");
    lists(&mut h, &Profile::Default, "local-queued", "starting");
    lists(&mut h, &verify(), "local-asks", "waiting");
    lists(&mut h, &verify(), "local-done", "idle");
    poll(&mut g, &mut h, 0);
    assert_eq!(
        told(&g),
        [
            ("hermes:local-cron".to_owned(), Running),
            ("hermes:local-queued".to_owned(), Running),
            ("hermes:verify:local-asks".to_owned(), Waiting),
            ("hermes:verify:local-done".to_owned(), Idle),
        ]
    );
    // A session that left the list is told no more: the hooks have it.
    h.streams
        .remove(&(Profile::Default, "local-cron".to_owned()));
    poll(&mut g, &mut h, 1_000);
    assert!(!told(&g).iter().any(|(id, _)| id == "hermes:local-cron"));
}

#[test]
fn a_card_makes_its_session_wait_where_hermes_says_working() {
    let (mut g, mut h) = start();
    // A set of questions: Hermes lists the session as working.
    let set = json!({"id": "srq-1", "method": "clarify", "params": {"session_id": "local-set",
        "questions": [{"qid": "q0", "question": "First?", "choices": ["a"], "multi_select": false}]}});
    h.stream(&verify(), "local-set").open.push(set);
    lists(&mut h, &verify(), "local-set", "working");
    poll(&mut g, &mut h, 0);
    assert_eq!(g.waiting().count(), 1);
    assert_eq!(told(&g), [("hermes:verify:local-set".to_owned(), Waiting)]);
}

#[test]
fn a_waiting_session_stays_waiting_however_long_it_waits() {
    let (mut g, mut h) = start();
    lists(&mut h, &verify(), "local-asks", "waiting");
    for minute in 0..60 {
        poll(&mut g, &mut h, minute * 60_000);
        assert_eq!(told(&g), [("hermes:verify:local-asks".to_owned(), Waiting)]);
        // Each poll vouches for it anew.
        assert_eq!(g.live_states().read_ms, minute * 60_000);
    }
    lists(&mut h, &verify(), "local-asks", "working");
    poll(&mut g, &mut h, 3_600_000);
    assert_eq!(told(&g), [("hermes:verify:local-asks".to_owned(), Running)]);
}

#[test]
fn nothing_is_told_but_what_a_poll_read_since_the_gateway_was_last_ready() {
    let mut g = Gateway::new(GatewayId::new("local"));
    let mut h = Hermes::default();
    lists(&mut h, &Profile::Default, "local-long", "working");
    let running = [("hermes:local-long".to_owned(), Running)];
    assert_eq!(told(&g), [], "never connected");
    identified(&mut g, "one");
    assert_eq!(told(&g), [], "ready, not yet polled");
    poll(&mut g, &mut h, 0);
    assert_eq!(told(&g), running);

    // The socket drops: what was listed is no longer known to hold.
    g.apply(Input::Lost(Failure::Closed(1006)), 100);
    assert_eq!(told(&g), []);
    g.apply(Input::Dialing, 200);
    assert_eq!(told(&g), []);
    // Back on the same instance: still nothing until a poll reads it.
    identified(&mut g, "one");
    assert_eq!(told(&g), []);
    poll(&mut g, &mut h, 300);
    assert_eq!(told(&g), running);

    // A restart: the new instance has to be read first.
    identified(&mut g, "two");
    assert_eq!(told(&g), []);
    poll(&mut g, &mut h, 400);
    assert_eq!(told(&g), running);

    // A gateway without the live connection is left to the hooks.
    let unsupported = Unsupported::TicketVerb;
    g.apply(Input::Denied(Denied::Unsupported(unsupported)), 500);
    assert_eq!(told(&g), []);
    assert_eq!(g.poll().map(|_| ()), None);
}

#[test]
fn a_profile_whose_list_failed_is_left_to_the_hooks() {
    let (mut g, mut h) = start();
    lists(&mut h, &Profile::Default, "local-a", "working");
    lists(&mut h, &verify(), "local-b", "working");
    poll(&mut g, &mut h, 0);
    assert_eq!(told(&g).len(), 2);
    h.failing.push(Step::List(verify()));
    poll(&mut g, &mut h, 1_000);
    assert_eq!(told(&g), [("hermes:local-a".to_owned(), Running)]);
    // The next poll that lists it tells it again.
    poll(&mut g, &mut h, 2_000);
    assert_eq!(told(&g).len(), 2);
}

#[test]
fn only_a_poll_that_counts_renews_what_is_told() {
    let (mut g, mut h) = start();
    lists(&mut h, &Profile::Default, "local-long", "working");
    poll(&mut g, &mut h, 1_000);
    assert_eq!(g.live_states().read_ms, 1_000);
    // A poll begun and never given back (a hung call) renews nothing, and
    // neither does anything else that happens meanwhile.
    let hung = g.poll().unwrap();
    g.apply(Input::Sent { request: "nothing" }, 50_000);
    assert_eq!(g.live_states().read_ms, 1_000);
    assert_eq!(told(&g).len(), 1);
    // A stale poll that comes back late is dropped and renews nothing.
    poll(&mut g, &mut h, 60_000);
    g.apply(Input::Polled(hung), 70_000);
    assert_eq!(g.live_states().read_ms, 60_000);
}

/// Against ticket 12's snapshots as merged: a session waiting on requests
/// whose event Hermes dropped is told waiting from its list alone, whether
/// or not the snapshot that finds its requests worked, and is told nothing
/// while the socket is gone, though its card stays.
#[test]
fn what_is_told_follows_the_list_whatever_the_snapshots_do() {
    let (mut g, mut h) = start();
    let s = "local-long";
    let approval = |id: &str| {
        json!({"kind": "approval", "prompt_id": id, "execution_generation": 1,
            "command": "synthetic command", "description": "synthetic approval",
            "choices": ["once", "deny"], "timeout_s": 20})
    };
    h.publish(&verify(), s, "approval.request", 1, approval("older"));
    for _ in 0..600 {
        h.publish(&verify(), s, "session.info", 1, json!({}));
    }
    h.keep_what_hermes_keeps(&verify(), s);
    h.publish(&verify(), s, "approval.request", 1, approval("newer"));
    let waiting = [("hermes:verify:local-long".to_owned(), Waiting)];

    // The snapshot fails: no card, but the list says it waits.
    h.failing.push(Step::Snapshot(g.session(&verify(), s)));
    poll(&mut g, &mut h, 0);
    assert_eq!(g.waiting().count(), 0);
    assert_eq!(told(&g), waiting);
    // The next one finds both requests; the one after needs no snapshot.
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), g.waiting().count()), (1, 2));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(h.snapshots(), 0);
    assert_eq!(told(&g), waiting);
    assert_eq!(g.live_states().read_ms, 2_000);

    // The socket drops: the cards stay, nothing is told.
    g.apply(Input::Lost(Failure::Closed(1006)), 3_000);
    assert_eq!(g.waiting().count(), 2);
    assert_eq!(told(&g), []);
    identified(&mut g, "one");
    assert_eq!(told(&g), []);
    // Back: the poll snapshots again, as ticket 12 does after a loss.
    poll(&mut g, &mut h, 10_000);
    assert_eq!(h.snapshots(), 1);
    assert_eq!(told(&g), waiting);
    assert_eq!(g.live_states().read_ms, 10_000);

    // Both settle elsewhere: the session idles and the cards close.
    for id in ["older", "newer"] {
        let body = json!({"prompt_id": id, "execution_generation": 1});
        h.publish(&verify(), s, "approval.settled", 1, body);
    }
    poll(&mut g, &mut h, 11_000);
    assert_eq!(g.waiting().count(), 0);
    assert_eq!(told(&g), [("hermes:verify:local-long".to_owned(), Idle)]);
}
