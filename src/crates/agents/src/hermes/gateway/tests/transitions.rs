//! What the cards do as Hermes changes, poll by poll: every way a card
//! opens, changes and closes, and what a poll asks of the gateway to know.
use super::super::*;
use super::fake::{identified, names, poll, ready, Hermes};
use super::fixture;
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

fn question(prompt: &str, turn: u64, timeout: Option<u64>) -> Value {
    let mut payload = json!({"kind": "clarify", "prompt_id": prompt, "execution_generation": turn,
        "question": "Which one?", "choices": ["a (Recommended)", "b"], "multi_select": false});
    if let Some(seconds) = timeout {
        payload["timeout_s"] = json!(seconds);
    }
    payload
}

fn asks(hermes: &mut Hermes, prompt: &str, turn: u64) {
    hermes.publish(
        &work(),
        S,
        "clarify.request",
        turn,
        question(prompt, turn, None),
    );
}

fn settles(hermes: &mut Hermes, prompt: &str, turn: u64) {
    let body = json!({"prompt_id": prompt, "execution_generation": turn});
    hermes.publish(&work(), S, "clarify.settled", turn, body);
}

fn ends(hermes: &mut Hermes, turn: u64, status: &str) {
    hermes.publish(
        &work(),
        S,
        "message.complete",
        turn,
        json!({ "status": status }),
    );
}

fn set(request: &str) -> Value {
    json!({"id": request, "method": "clarify", "params": {"session_id": S, "questions": [
        {"qid": "q0", "question": "First?", "choices": ["a", "b"], "multi_select": false}]}})
}

fn closed(effects: &[Effect]) -> Vec<(&str, &Closed)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Close { id, why, .. } => Some((id.rsplit(':').next().unwrap(), why)),
            _ => None,
        })
        .collect()
}

fn opened(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Open(pending) => Some(pending.ask.key()),
            _ => None,
        })
        .collect()
}

#[test]
fn what_already_waits_is_found_without_subscribing() {
    let (mut g, mut h) = start();
    h.publish(&work(), S, "message.start", 1, json!({}));
    h.publish(
        &work(),
        S,
        "clarify.request",
        1,
        question("p1", 1, Some(20)),
    );
    let effects = poll(&mut g, &mut h, 0);
    assert_eq!(opened(&effects), ["p1"]);
    // One list for each served profile, then the session's stream read
    // twice: where it is, and what it holds from its start. No snapshot.
    let at = g.session(&work(), S).unwrap();
    let start = Cursor {
        epoch: "epoch-1".into(),
        seq: 0,
    };
    assert_eq!(
        h.asked,
        [
            Step::List(Profile::Default),
            Step::List(work()),
            Step::Since {
                session: at.clone(),
                cursor: None
            },
            Step::Since {
                session: at.clone(),
                cursor: Some(start.clone())
            },
        ]
    );
    // The exact calls, on a socket for the whole host and on a profile's own.
    assert_eq!(
        h.asked[1].command(Scope::Host).frame(3),
        json!({"jsonrpc": "2.0", "id": 3, "method": "session.active_list", "params": {"profile": "verify"}})
    );
    assert_eq!(
        h.asked[3].command(Scope::Profile).frame(4),
        json!({"jsonrpc": "2.0", "id": 4, "method": "session.events.since", "params": {
            "session_id": S, "last_sequence": 0, "replay_epoch": "epoch-1"}})
    );
    assert_eq!(
        Step::Snapshot(at).command(Scope::Host).frame(5),
        json!({"jsonrpc": "2.0", "id": 5, "method": "session.resume", "params": {
            "session_id": S, "profile": "verify"}})
    );
    // A session that neither works nor waits and shows no card is not read.
    settles(&mut h, "p1", 1);
    ends(&mut h, 1, "complete");
    assert_eq!(closed(&poll(&mut g, &mut h, 1)), [("p1", &Closed::Settled)]);
    poll(&mut g, &mut h, 2);
    assert_eq!(h.asked, [Step::List(Profile::Default), Step::List(work())]);
}

#[test]
fn a_request_asked_and_settled_within_one_reading_is_never_shown() {
    let (mut g, mut h) = start();
    h.publish(&work(), S, "message.start", 1, json!({}));
    asks(&mut h, "p1", 1);
    settles(&mut h, "p1", 1);
    assert_eq!(poll(&mut g, &mut h, 0), []);
    // Asked again under the same id after it settled, it waits again.
    asks(&mut h, "p1", 1);
    assert_eq!(opened(&poll(&mut g, &mut h, 1)), ["p1"]);
}

#[test]
fn a_turn_that_ended_takes_only_its_own_cards() {
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    asks(&mut h, "new", 2);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["new", "old"]);
    // Turn 1 ends without a word about its question; turn 2 still waits.
    ends(&mut h, 1, "interrupted");
    h.stream(&work(), S).prompts.remove(0);
    assert_eq!(
        closed(&poll(&mut g, &mut h, 1)),
        [("old", &Closed::Interrupted)]
    );
    assert_eq!(names(&g), ["new"]);
    // A request of a turn that is over is not shown, wherever its request
    // event stands among the events read.
    asks(&mut h, "stray", 1);
    h.stream(&work(), S).prompts.pop();
    assert_eq!(poll(&mut g, &mut h, 2), []);
    ends(&mut h, 2, "complete");
    h.stream(&work(), S).prompts.clear();
    assert_eq!(
        closed(&poll(&mut g, &mut h, 3)),
        [("new", &Closed::TurnEnded)]
    );

    // A turn that started takes the requests of the turns before it, even
    // where nothing was said of their end: Hermes runs one turn at a time.
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    h.publish(&work(), S, "message.start", 2, json!({}));
    asks(&mut h, "new", 2);
    let effects = poll(&mut g, &mut h, 1);
    assert_eq!(closed(&effects), [("old", &Closed::TurnEnded)]);
    assert_eq!(names(&g), ["new"]);
}

#[test]
fn nothing_closes_a_card_but_hermes() {
    let (mut g, mut h) = start();
    h.publish(
        &work(),
        S,
        "clarify.request",
        1,
        question("p1", 1, Some(20)),
    );
    let approval = json!({"kind": "approval", "prompt_id": "a1", "execution_generation": 1,
        "command": "rm -r x", "description": "delete x", "choices": ["once", "deny"], "timeout_s": 20});
    h.publish(&work(), S, "approval.request", 1, approval);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["a1", "p1"]);
    // Long past the 20 s both said: Hermes still lists them, so they show.
    // An approval's wait ends in Hermes without a word until its turn ends.
    assert_eq!(poll(&mut g, &mut h, 86_400_000), []);
    assert_eq!(names(&g), ["a1", "p1"]);
    assert_eq!(g.poll_ms(), BUSY_POLL_MS);
    // The turn ends: the approval goes with it, settled only now.
    settles(&mut h, "p1", 1);
    let late = json!({"prompt_id": "a1", "execution_generation": 1});
    h.publish(&work(), S, "approval.settled", 1, late);
    ends(&mut h, 1, "complete");
    let effects = poll(&mut g, &mut h, 86_400_001);
    assert_eq!(
        closed(&effects),
        [("a1", &Closed::Settled), ("p1", &Closed::Settled)]
    );
}

#[test]
fn an_answer_that_came_too_late_closes_its_card() {
    let late = fixture("question-answered-elsewhere");
    let refused = fixture("question-peek-without-subscribing");
    let invalid = fixture("approval-invalid-answers");
    for (reply, why) in [
        (&late["notes"]["other_answer"], Some(Closed::Answered)),
        (
            &late["notes"]["hud_late_answer"],
            Some(Closed::AlreadyResolved),
        ),
        (
            &late["notes"]["hud_answer_after_turn_end"],
            Some(Closed::StaleGeneration),
        ),
        (&refused["notes"]["answer_unsubscribed"], None),
        (&invalid["notes"]["unknown_choice"], None),
    ] {
        let (mut g, mut h) = start();
        asks(&mut h, "p1", 1);
        poll(&mut g, &mut h, 0);
        let request = g.waiting().next().unwrap().id();
        // A poll under way when the reply comes still says the request waits.
        let mut under_way = g.poll().unwrap();
        super::fake::ask(&mut under_way, &mut h);
        assert_eq!(g.apply(Input::Sent { request: &request }, 10), []);
        let effects = g.apply(
            Input::Responded {
                request: &request,
                reply,
            },
            20,
        );
        match why {
            Some(why) => {
                assert_eq!(closed(&effects), [("p1", &why)], "{reply}");
                // Poll at once; the poll from before the reply is dropped,
                // so the card it would show again stays closed.
                assert_eq!(effects.last(), Some(&Effect::Poll));
                assert_eq!(g.apply(Input::Polled(under_way), 30), []);
                assert!(names(&g).is_empty());
            }
            // Refused for another reason: the request still waits, so its
            // card comes back with the gateway's reason.
            None => {
                let pending = g.pending(&request).unwrap().clone();
                let why = refusal(reply);
                assert!(why.is_some());
                assert_eq!(effects, [Effect::Unanswered { pending, why }]);
            }
        }
    }
    assert!(Refusal::PermissionDenied.retryable() && !Refusal::InvalidParams.retryable());
}

#[test]
fn a_restart_closes_every_card_and_a_dropped_socket_closes_none() {
    let (mut g, mut h) = start();
    h.publish(
        &work(),
        S,
        "clarify.request",
        1,
        question("p1", 1, Some(20)),
    );
    h.stream(&work(), S).open = vec![set("srq-1")];
    poll(&mut g, &mut h, 0);
    let request = g.session(&work(), S).unwrap().request_id("p1");
    g.apply(Input::Sent { request: &request }, 0);

    // The socket drops: both still wait in Hermes. The answer on its way
    // may be lost, so its card is shown again.
    let effects = g.apply(Input::Lost(Failure::Closed(1006)), 5);
    let pending = g.pending(&request).unwrap().clone();
    // Both cards stay, marked as out of reach (ticket 27).
    let both: Vec<Pending> = g.waiting().cloned().collect();
    assert_eq!(
        effects,
        [
            Effect::Unanswered {
                pending: pending.clone(),
                why: None
            },
            Effect::Connection(Connection::Failed(Failure::Closed(1006))),
            Effect::Marked(both[0].clone()),
            Effect::Marked(both[1].clone()),
        ]
    );
    assert_eq!(both[0], pending);
    assert_eq!(names(&g), ["p1", "srq-1"]);

    // The same instance again: nothing was lost, and the poll that finds
    // both still waiting takes their marks away.
    assert!(closed(&identified(&mut g, "one")).is_empty());
    assert_eq!(
        poll(&mut g, &mut h, 6),
        [
            Effect::Unmarked(both[0].clone()),
            Effect::Unmarked(both[1].clone())
        ]
    );
    assert_eq!(poll(&mut g, &mut h, 7), []);
    assert_eq!(names(&g).len(), 2);

    // Another instance is a restart, even on the same port.
    g.apply(Input::Lost(Failure::Closed(1012)), 7);
    let effects = identified(&mut g, "two");
    assert_eq!(
        closed(&effects),
        [
            ("p1", &Closed::GatewayRestarted),
            ("srq-1", &Closed::GatewayRestarted)
        ]
    );
    assert!(matches!(g.connection(), Connection::Ready(instance) if instance.id == "two"));
    assert_eq!(g.statuses(), []);

    // A gateway that turns out to lack the connection closes them as well.
    let (mut g, mut h) = start();
    asks(&mut h, "p1", 1);
    poll(&mut g, &mut h, 0);
    let effects = g.apply(
        Input::Denied(Denied::Unsupported(Unsupported::TicketVerb)),
        1,
    );
    assert_eq!(closed(&effects), [("p1", &Closed::Unsupported)]);
    assert!(g.poll().is_none());
}

#[test]
fn a_gateway_waiting_to_restart_can_still_be_answered() {
    let mut answer = ready("one", &["default"]);
    answer["state"] = json!("draining");
    let home = "/synthetic";
    assert_eq!(identify(&answer, home), Identified::Wait("draining".into()));
    answer["drain_reason"] = json!("restart_wait");
    assert!(matches!(identify(&answer, home), Identified::Ready(instance) if instance.draining));
    answer["state"] = json!("stopped");
    assert_eq!(
        identify(&answer, home),
        Identified::Failed(Failure::Invalid("invalid_runtime_state"))
    );
    // Only this machine's own gateway is ever dialled, and only over plain
    // HTTP: any other origin is left to the hooks.
    for origin in [
        "https://127.0.0.1:5000",
        "https://[::1]:5000",
        "http://192.168.1.4:5000",
        "http://example.com:5000",
        "http://127.0.0.1",
        "ftp://127.0.0.1:5000",
        "http://127.0.0.1:5000/path",
    ] {
        let mut answer = ready("one", &["default"]);
        answer["api_origin"] = json!(origin);
        assert_eq!(
            identify(&answer, home),
            Identified::Unsupported(Unsupported::Origin),
            "{origin}"
        );
    }
    let mut answer = ready("one", &["default"]);
    answer["api_origin"] = json!("http://[::1]:5000/");
    assert!(
        matches!(identify(&answer, home), Identified::Ready(instance) if instance.port == 5000)
    );
}

#[test]
fn a_set_of_questions_follows_what_is_open_now() {
    let (mut g, mut h) = start();
    // A session waiting on a set of questions says it works, not waits.
    h.stream(&work(), S).running = true;
    h.stream(&work(), S).open = vec![set("srq-1")];
    let effects = poll(&mut g, &mut h, 0);
    assert_eq!(opened(&effects), ["srq-1"]);
    assert_eq!(h.snapshots(), 0);
    let at = g.session(&work(), S).unwrap();
    assert_eq!(g.status(&at), Some(Status::Waiting));
    let pending = g.waiting().next().unwrap().clone();
    assert_eq!(pending.wait_ms, None);

    // Nothing replies to its answer: sending it asks for a poll at once.
    assert_eq!(
        g.apply(
            Input::Sent {
                request: &pending.id()
            },
            1_000
        ),
        [Effect::Poll]
    );
    assert_eq!(poll(&mut g, &mut h, 1_100), []);
    // Still listed 5 s on, the answer was dropped: the card is shown
    // again, once, and nothing closed.
    let unanswered = Effect::Unanswered {
        pending: pending.clone(),
        why: None,
    };
    assert_eq!(poll(&mut g, &mut h, 6_000), [unanswered]);
    assert_eq!(poll(&mut g, &mut h, 7_000), []);

    // Hermes withdraws it, whatever its word for why: it is over when
    // `open_requests` no longer lists it.
    for (reason, why) in [
        ("resolved", Closed::Settled),
        ("timeout", Closed::TimedOut),
        ("interrupted", Closed::Interrupted),
        ("session_closed", Closed::Withdrawn("session_closed".into())),
    ] {
        let (mut g, mut h) = start();
        h.stream(&work(), S).running = true;
        h.stream(&work(), S).open = vec![set("srq-1")];
        poll(&mut g, &mut h, 0);
        h.stream(&work(), S).open.clear();
        let cancel = json!({"id": "srq-1", "method": "clarify", "reason": reason});
        h.publish(&work(), S, "request.cancel", 1, cancel);
        assert_eq!(closed(&poll(&mut g, &mut h, 1)), [("srq-1", &why)]);
    }
    // A password or a secret Hermes asks its viewers for is never shown.
    let (mut g, mut h) = start();
    h.stream(&work(), S).running = true;
    h.stream(&work(), S).open =
        vec![json!({"id": "srq-2", "method": "sudo", "params": {"session_id": S}})];
    assert_eq!(poll(&mut g, &mut h, 0), []);
}

#[test]
fn a_snapshot_is_taken_only_where_the_events_cannot_say() {
    let (mut g, mut h) = start();
    asks(&mut h, "p1", 1);
    for _ in 0..20 {
        h.publish(&work(), S, "session.info", 1, json!({}));
    }
    // The request's event is gone from the stream while it still waits.
    h.drop_through(&work(), S, 10);
    let effects = poll(&mut g, &mut h, 0);
    assert_eq!(
        h.asked.last(),
        Some(&Step::Snapshot(g.session(&work(), S).unwrap()))
    );
    assert_eq!(opened(&effects), ["p1"]);
    // While that lasts the events never account for the card. It stays,
    // and no second snapshot is taken so soon after the first.
    assert_eq!(poll(&mut g, &mut h, 1_000), []);
    assert_eq!((h.snapshots(), names(&g)), (0, vec!["p1".to_owned()]));

    // A second request, whose event the stream still holds, shows too.
    asks(&mut h, "p2", 1);
    assert_eq!(opened(&poll(&mut g, &mut h, 2_000)), ["p2"]);
    assert_eq!(names(&g), ["p1", "p2"]);

    // Both settle, and the stream drops everything it held. Hermes says
    // the session works, so nothing of the kind waits in it: both close,
    // with no snapshot and no event to say why.
    settles(&mut h, "p1", 1);
    settles(&mut h, "p2", 1);
    h.drop_through(&work(), S, 24);
    h.stream(&work(), S).running = true;
    let effects = poll(&mut g, &mut h, 3_000);
    assert_eq!(
        closed(&effects),
        [("p1", &Closed::Gone), ("p2", &Closed::Gone)]
    );
    assert_eq!(h.snapshots(), 0);

    // A session that only works is never subscribed to, and a stream of
    // which nothing can be read still tells its sets of questions.
    h.stream(&work(), S).open = vec![set("srq-9")];
    assert_eq!(opened(&poll(&mut g, &mut h, 4_000)), ["srq-9"]);
    assert_eq!(h.snapshots(), 0);
}

#[test]
fn a_request_that_cannot_be_shown_gets_no_card_and_no_snapshot() {
    let (mut g, mut h) = start();
    // A kind of prompt Hermes may add later, and a question without its text.
    let unknown = json!({"kind": "passphrase", "prompt_id": "x1", "execution_generation": 1});
    h.publish(&work(), S, "clarify.request", 1, unknown);
    let mut blank = question("x2", 1, None);
    blank["question"] = json!("");
    h.publish(&work(), S, "clarify.request", 1, blank);
    // The session waits, and the events read say on what: no card, and no
    // snapshot, however often it is polled.
    for now in [0, 1_000, 2_000] {
        assert_eq!(poll(&mut g, &mut h, now), []);
        assert_eq!(h.snapshots(), 0);
    }
    assert_eq!(
        g.status(&g.session(&work(), S).unwrap()),
        Some(Status::Waiting)
    );
    // One that can be shown, asked beside them, still shows.
    asks(&mut h, "p1", 1);
    assert_eq!(opened(&poll(&mut g, &mut h, 3_000)), ["p1"]);
    assert_eq!(h.snapshots(), 0);
}

#[test]
fn a_poll_that_could_not_tell_leaves_the_cards_alone() {
    let (mut g, mut h) = start();
    asks(&mut h, "p1", 1);
    poll(&mut g, &mut h, 0);
    let at = g.session(&work(), S).unwrap();
    // The profile's list fails; then the session's read fails.
    for failing in [
        Step::List(work()),
        Step::Since {
            session: at.clone(),
            cursor: None,
        },
    ] {
        h.failing.push(failing);
        settles(&mut h, "p1", 1);
        assert_eq!(poll(&mut g, &mut h, 1), []);
        assert_eq!(names(&g), ["p1"]);
        assert_eq!(g.status(&at), Some(Status::Waiting));
    }
    // The next poll asks everything again and finds it settled.
    assert_eq!(closed(&poll(&mut g, &mut h, 2)), [("p1", &Closed::Settled)]);
    // Nor does a poll that ends while the gateway is being dialled again.
    asks(&mut h, "p3", 1);
    let mut under_way = g.poll().unwrap();
    super::fake::ask(&mut under_way, &mut h);
    g.apply(Input::Dialing, 3);
    assert_eq!(g.apply(Input::Polled(under_way), 3), []);
    identified(&mut g, "one");
    assert_eq!(opened(&poll(&mut g, &mut h, 3)), ["p3"]);
    settles(&mut h, "p3", 1);
    poll(&mut g, &mut h, 3);
    // A poll that was never run to its end changes nothing either.
    asks(&mut h, "p2", 1);
    let unfinished = g.poll().unwrap();
    assert_eq!(unfinished.steps().len(), 2);
    assert_eq!(g.apply(Input::Polled(unfinished), 3), []);
    assert!(names(&g).is_empty());
}

#[test]
fn a_session_that_left_the_live_list_is_over_with_its_requests() {
    let (mut g, mut h) = start();
    asks(&mut h, "p1", 1);
    h.stream(&work(), "local-bbbb").running = true;
    poll(&mut g, &mut h, 0);
    assert_eq!(g.statuses().len(), 2);
    h.streams.remove(&(work(), S.to_owned()));
    let effects = poll(&mut g, &mut h, 1);
    assert_eq!(closed(&effects), [("p1", &Closed::SessionEnded)]);
    let Effect::Close { session, .. } = &effects[0] else {
        panic!("a close");
    };
    assert_eq!(session.hud_id(), "hermes:verify:local-aaaa");
    assert_eq!(
        g.statuses(),
        [(g.session(&work(), "local-bbbb").unwrap(), Status::Working)]
    );

    // A turn that is over takes its card even where no event says it ended:
    // no turn runs in a session that neither works nor waits.
    let (mut g, mut h) = start();
    asks(&mut h, "p1", 1);
    poll(&mut g, &mut h, 0);
    h.stream(&work(), S).status = Some("idle".into());
    assert_eq!(
        closed(&poll(&mut g, &mut h, 1)),
        [("p1", &Closed::TurnEnded)]
    );
}

#[test]
fn the_same_ids_on_two_gateways_or_profiles_are_different_requests() {
    let mut ids = std::collections::BTreeSet::new();
    let macbook = GatewayId::new(Machine::This, Profile::Default);
    let mini = GatewayId::new(super::far(super::INSTALL), Profile::Default);
    for id in [macbook, mini] {
        let mut gateway = Gateway::new(id.clone());
        identified(&mut gateway, "one");
        let mut hermes = Hermes::default();
        for profile in [Profile::Default, work()] {
            hermes.publish(&profile, S, "clarify.request", 1, question("p1", 1, None));
        }
        for effect in poll(&mut gateway, &mut hermes, 0) {
            let Effect::Open(pending) = effect else {
                panic!("two cards");
            };
            assert_eq!(pending.session.gateway, id);
            ids.insert(pending.id());
        }
        assert_eq!(gateway.waiting().count(), 2);
    }
    let expected = [
        "hermes:aaeda597d860920c/default:default:local-aaaa:p1",
        "hermes:aaeda597d860920c/default:verify:local-aaaa:p1",
        "hermes:local:default:local-aaaa:p1",
        "hermes:local:verify:local-aaaa:p1",
    ];
    assert_eq!(ids.iter().map(String::as_str).collect::<Vec<_>>(), expected);
}

#[test]
fn hints_and_the_timer_say_when_to_poll() {
    let event = |kind: &str, session: &str| {
        json!({"jsonrpc": "2.0", "method": "event",
        "params": {"type": kind, "session_id": session, "payload": {}}})
    };
    for kind in ["sessions.changed", "platforms.changed"] {
        assert!(hints(&event(kind, "")));
    }
    assert!(hints(&event("clarify.request", S)) && hints(&event("message.complete", S)));
    assert!(hints(&set("srq-1")));
    assert!(!hints(&event("gateway.ready", "")) && !hints(&event("message.delta", S)));
    assert!(!hints(&json!({"jsonrpc": "2.0", "id": 4, "result": {}})));
    // Recorded: `sessions.changed` does arrive on an unsubscribed socket.
    let doc = fixture("session-list");
    let frames = doc["clients"]["hud"].as_array().unwrap();
    let changed = |entry: &&Value| entry["frame"]["params"]["type"] == "sessions.changed";
    assert!(frames
        .iter()
        .filter(changed)
        .all(|entry| hints(&entry["frame"])));
    assert!(frames.iter().any(|entry| changed(&entry)));

    // The timer never stops: slow while nothing seems to happen, since a
    // hint or a hook can be missed, and fast while anything does.
    let (mut g, mut h) = start();
    assert_eq!(g.poll_ms(), IDLE_POLL_MS);
    h.stream(&work(), S).status = Some("idle".into());
    poll(&mut g, &mut h, 0);
    assert_eq!(g.poll_ms(), IDLE_POLL_MS);
    for status in ["starting", "working", "waiting"] {
        h.stream(&work(), S).status = Some(status.into());
        poll(&mut g, &mut h, 0);
        assert_eq!(g.poll_ms(), BUSY_POLL_MS, "{status}");
    }
}

#[test]
fn many_sessions_cost_a_read_only_where_something_may_wait() {
    let (mut g, mut h) = start();
    // More sessions than Hermes keeps streams for (64), all idle but two.
    for index in 0..80 {
        h.stream(&work(), &format!("local-{index:04}")).status = Some("idle".into());
    }
    h.stream(&work(), "local-0007").status = Some("starting".into());
    h.publish(
        &work(),
        "local-0042",
        "clarify.request",
        3,
        question("p1", 3, None),
    );
    h.stream(&work(), "local-0042").status = None;
    assert_eq!(opened(&poll(&mut g, &mut h, 0)), ["p1"]);
    // Two lists, and two reads of the one session that waits.
    assert_eq!(h.asked.len(), 4);
    assert_eq!(g.statuses().len(), 80);
}
