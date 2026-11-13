//! The orders of delivery an independent review of this model found it got
//! wrong: a snapshot that arrives after newer events, a stream that starts
//! over, a set of questions another viewer answers, a completion read late,
//! and a duration taken for a deadline. The tests keep the review's names.
use super::super::*;
use serde_json::{json, Value};

const S: &str = "synthetic-session";

fn profile() -> Profile {
    Profile::named("verify")
}

fn model() -> Gateway {
    Gateway::new(GatewayId::new("synthetic-gateway"))
}

fn payload(id: &str, generation: u64) -> Value {
    json!({"kind": "clarify", "prompt_id": id, "execution_generation": generation,
        "question": "Synthetic question", "choices": ["a (Recommended)", "b"], "multi_select": false})
}

fn event(kind: &str, generation: u64, seq: u64, body: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": "event", "params": {
        "type": kind, "session_id": S, "execution_generation": generation,
        "replay_epoch": "epoch-1", "seq": seq, "payload": body}})
}

fn ask(id: &str, generation: u64, seq: u64) -> Value {
    event("clarify.request", generation, seq, payload(id, generation))
}

fn settled(id: &str, generation: u64, seq: u64) -> Value {
    let body = json!({"prompt_id": id, "execution_generation": generation});
    event("clarify.settled", generation, seq, body)
}

fn feed(g: &mut Gateway, frame: &Value) -> Vec<Effect> {
    g.apply(
        Input::Frame {
            run: g.run(),
            profile: &profile(),
            frame,
        },
        0,
    )
}

fn snapshot(prompts: Value, batches: Value, seq: u64) -> Value {
    json!({"session_id": S, "replay_epoch": "epoch-1", "last_sequence": seq,
        "subscription_id": "synthetic-sub", "prompts": prompts, "open_requests": batches})
}

fn resume(g: &mut Gateway, result: &Value, now: i64) -> Vec<Effect> {
    g.apply(
        Input::Resumed {
            run: g.run(),
            profile: &profile(),
            result,
        },
        now,
    )
}

fn replay(g: &mut Gateway, result: &Value, now: i64) -> Vec<Effect> {
    g.apply(
        Input::Replayed {
            run: g.run(),
            profile: &profile(),
            session: S,
            result,
        },
        now,
    )
}

fn names(g: &Gateway) -> Vec<String> {
    g.waiting().map(|p| p.ask.key().to_owned()).collect()
}

fn batch(answers: Value) -> Value {
    json!({"id": "srq-synthetic", "method": "clarify", "params": {
        "session_id": S, "questions": [
            {"qid": "q0", "question": "Synthetic first", "choices": ["a", "b"], "multi_select": false},
            {"qid": "q1", "question": "Synthetic second", "choices": ["a", "b"], "multi_select": false}],
        "answers": answers}})
}

fn identified(g: &mut Gateway, instance: &str) {
    let answer = json!({"runtime_protocol": 1, "state": "ready", "instance_id": instance,
        "authority_epoch": 1, "api_origin": "http://127.0.0.1:54321",
        "capabilities": [SESSION_AUTHORITY],
        "served_profiles": [{"profile_id": "synthetic", "home": "/synthetic"}]});
    g.apply(
        Input::Identified {
            answer: &answer,
            home: "/synthetic",
        },
        0,
    );
}

#[test]
fn stale_snapshot_must_not_close_newer_live_card_or_regress_cursor() {
    let mut g = model();
    feed(&mut g, &ask("new", 2, 12));
    let effects = resume(&mut g, &snapshot(json!([]), json!([]), 10), 100);
    assert_eq!(
        names(&g),
        ["new"],
        "older snapshot closed a live card: {effects:?}"
    );
    assert_eq!(g.cursor(&g.session(&profile(), S)).unwrap().seq, 12);
    // A snapshot taken after the card was seen does say it is over.
    let effects = resume(&mut g, &snapshot(json!([]), json!([]), 12), 200);
    assert!(names(&g).is_empty(), "{effects:?}");
}

#[test]
fn a_snapshot_from_before_events_that_were_never_read_is_not_merged() {
    let mut g = model();
    let list = json!({"sessions": [{"id": S, "status": "working"}]});
    g.apply(
        Input::Listed {
            run: g.run(),
            profile: &profile(),
            result: &list,
        },
        0,
    );
    // A long turn outran the stream: the reading goes on from event 900,
    // and what happened before it was never read.
    let cut = json!({"replay_epoch": "epoch-1", "last_sequence": 900, "events": [],
        "snapshot_required": true, "open_requests": []});
    replay(&mut g, &cut, 0);
    replay(&mut g, &cut, 0);
    assert_eq!(g.cursor(&g.session(&profile(), S)).unwrap().seq, 900);
    // A snapshot from event 500 may list what settled at 501, unseen.
    let effects = resume(
        &mut g,
        &snapshot(json!([payload("early", 1)]), json!([]), 500),
        100,
    );
    assert!(names(&g).is_empty(), "{effects:?}");
    // One from where the reading went on is whole.
    resume(
        &mut g,
        &snapshot(json!([payload("early", 1)]), json!([]), 900),
        200,
    );
    assert_eq!(names(&g), ["early"]);
}

#[test]
fn settlement_seen_before_snapshot_must_prevent_stale_open() {
    let mut g = model();
    feed(&mut g, &settled("old", 1, 11));
    let effects = resume(
        &mut g,
        &snapshot(json!([payload("old", 1)]), json!([]), 10),
        100,
    );
    assert!(
        names(&g).is_empty(),
        "settled request reopened: {effects:?}"
    );
    // The same for a set of questions withdrawn before it was ever shown.
    let cancel = json!({"id": "srq-synthetic", "method": "clarify", "reason": "resolved"});
    feed(&mut g, &event("request.cancel", 1, 12, cancel));
    let effects = resume(
        &mut g,
        &snapshot(json!([]), json!([batch(json!({}))]), 10),
        100,
    );
    assert!(names(&g).is_empty(), "withdrawn set reopened: {effects:?}");
}

#[test]
fn turn_end_must_not_erase_retirement_of_newer_generation() {
    let mut g = model();
    feed(&mut g, &ask("new", 2, 10));
    feed(&mut g, &settled("new", 2, 11));
    feed(
        &mut g,
        &event("message.complete", 1, 12, json!({"status": "complete"})),
    );
    let effects = resume(
        &mut g,
        &snapshot(json!([payload("new", 2)]), json!([]), 10),
        100,
    );
    assert!(
        names(&g).is_empty(),
        "old completion erased new retirement: {effects:?}"
    );
    // Once its own turn ends, no reading of that turn shows it either.
    feed(
        &mut g,
        &event("message.complete", 2, 13, json!({"status": "complete"})),
    );
    let effects = resume(
        &mut g,
        &snapshot(json!([payload("new", 2)]), json!([]), 10),
        100,
    );
    assert!(names(&g).is_empty(), "{effects:?}");
}

#[test]
fn late_old_instance_snapshot_must_not_repopulate_restarted_gateway() {
    let mut g = model();
    identified(&mut g, "instance-1");
    let asked_in = g.run();
    feed(&mut g, &ask("old", 1, 10));
    identified(&mut g, "instance-2");
    assert!(names(&g).is_empty());
    assert_ne!(g.run(), asked_in);
    // Asked of instance 1, answered after instance 2 was identified.
    let result = snapshot(json!([payload("old", 1)]), json!([]), 10);
    let late = Input::Resumed {
        run: asked_in,
        profile: &profile(),
        result: &result,
    };
    let effects = g.apply(late, 100);
    assert!(
        names(&g).is_empty(),
        "old instance snapshot reopened request: {effects:?}"
    );
    assert!(g.statuses().is_empty());
    // The same for its frames, its lists, its replays and its replies.
    let frame = ask("old", 1, 11);
    let list = json!({"sessions": [{"id": S, "status": "waiting"}]});
    let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"status": "resolved"}});
    for late in [
        Input::Frame {
            run: asked_in,
            profile: &profile(),
            frame: &frame,
        },
        Input::Listed {
            run: asked_in,
            profile: &profile(),
            result: &list,
        },
        Input::Replayed {
            run: asked_in,
            profile: &profile(),
            session: S,
            result: &result,
        },
        Input::Responded {
            run: asked_in,
            request: "any",
            reply: &reply,
        },
    ] {
        assert_eq!(g.apply(late, 100), []);
    }
    assert!(g.statuses().is_empty());
}

#[test]
fn epoch_rollover_on_live_event_must_request_reconciliation() {
    let mut g = model();
    let at = g.session(&profile(), S);
    feed(&mut g, &ask("old", 1, 10));
    let mut changed = ask("new", 2, 1);
    changed["params"]["replay_epoch"] = json!("epoch-2");
    let effects = feed(&mut g, &changed);
    let wanted = Effect::Resume {
        session: at.clone(),
    };
    assert!(
        effects.contains(&wanted),
        "no snapshot requested: {effects:?}"
    );
    // Until a snapshot comes, every reading of the session asks again.
    let tail = json!({"replay_epoch": "epoch-2", "last_sequence": 1, "events": [],
        "snapshot_required": false, "open_requests": []});
    assert_eq!(replay(&mut g, &tail, 100), std::slice::from_ref(&wanted));
    let list = json!({"sessions": [{"id": S, "status": "waiting"}]});
    let listed = g.apply(
        Input::Listed {
            run: g.run(),
            profile: &profile(),
            result: &list,
        },
        100,
    );
    assert_eq!(listed, [wanted]);
    // What the old stream says now arrives late and changes nothing.
    assert_eq!(feed(&mut g, &settled("new", 2, 11)), []);
    assert_eq!(names(&g), ["new", "old"]);
    // The snapshot of the new stream settles it: the old card is over.
    let mut now = snapshot(json!([payload("new", 2)]), json!([]), 1);
    now["replay_epoch"] = json!("epoch-2");
    let effects = resume(&mut g, &now, 200);
    assert_eq!(names(&g), ["new"], "{effects:?}");
    assert_eq!(replay(&mut g, &tail, 300), []);
}

#[test]
fn batch_reread_must_update_locks_and_emit_changed_card() {
    let mut g = model();
    resume(
        &mut g,
        &snapshot(json!([]), json!([batch(json!({}))]), 10),
        0,
    );
    let changed = snapshot(
        json!([]),
        json!([batch(json!({"q0": "a", "q1": null}))]),
        11,
    );
    let effects = resume(&mut g, &changed, 100);
    let pending = g.waiting().next().unwrap().clone();
    let Ask::Batch(set) = &pending.ask else {
        panic!("a set of questions");
    };
    assert_eq!(set.questions[0].locked, Some(Locked::Answered("a".into())));
    assert_eq!(set.questions[1].locked, Some(Locked::Skipped));
    // The same request, shown as it is now: no second card.
    assert_eq!(effects, [Effect::Changed(pending)]);
    assert_eq!(g.waiting().count(), 1);
    // Read again unchanged, or from an older place, nothing changes.
    assert_eq!(resume(&mut g, &changed, 200), []);
    assert_eq!(
        resume(
            &mut g,
            &snapshot(json!([]), json!([batch(json!({}))]), 10),
            300
        ),
        []
    );
}

#[test]
fn batch_card_must_distinguish_locked_and_unanswered_questions() {
    let read = |answers: Value| {
        let ask = Ask::batch("srq-synthetic", "clarify", &batch(answers)["params"]).unwrap();
        let session = model().session(&profile(), S);
        Pending {
            session,
            ask,
            deadline_ms: None,
            recheck_ms: None,
        }
        .view(0)
    };
    let (unlocked, locked) = (read(json!({})), read(json!({"q0": "a", "q1": null})));
    assert_ne!(
        unlocked, locked,
        "locked answer is invisible to ticket 14's card"
    );
    assert_eq!(unlocked["questions"][0]["lock"], Value::Null);
    assert_eq!(
        locked["questions"][0]["lock"],
        json!({"state": "answered", "answer": "a"})
    );
    assert_eq!(locked["questions"][1]["lock"], json!({"state": "skipped"}));
}

#[test]
fn old_completion_must_not_mark_new_turn_idle() {
    let mut g = model();
    feed(&mut g, &event("message.start", 2, 10, json!({})));
    feed(
        &mut g,
        &event("message.complete", 1, 11, json!({"status": "complete"})),
    );
    assert_eq!(
        g.statuses()[0].1,
        Status::Working,
        "ticket 16 would report the new turn idle"
    );
    // Nor does a start read late make an ended turn work again.
    feed(
        &mut g,
        &event("message.complete", 2, 12, json!({"status": "complete"})),
    );
    feed(&mut g, &event("message.start", 1, 13, json!({})));
    assert_eq!(g.statuses()[0].1, Status::Idle);
}

#[test]
fn replayed_duration_must_not_be_treated_as_known_absolute_deadline() {
    let mut g = model();
    let at = g.session(&profile(), S);
    let mut frame = ask("old", 1, 10);
    frame["params"]["payload"]["timeout_s"] = json!(20);
    let read = json!({"replay_epoch": "epoch-1", "last_sequence": 10,
        "events": [frame["params"]], "snapshot_required": false, "open_requests": []});
    replay(&mut g, &read, 100_000);
    let pending = g.waiting().next().unwrap();
    assert_eq!(
        pending.deadline_ms, None,
        "no creation timestamp: the expiration is unknown"
    );
    assert_eq!(pending.recheck_ms, Some(120_000));
    assert_eq!((g.next_deadline(), g.next_wake()), (None, Some(120_000)));
    // The recheck closes nothing. It asks Hermes, and sets itself again.
    assert_eq!(g.apply(Input::Tick, 119_999), []);
    assert_eq!(
        g.apply(Input::Tick, 120_000),
        [Effect::Resume { session: at }]
    );
    assert_eq!(names(&g), ["old"]);
    assert_eq!(g.next_wake(), Some(140_000));
    // Hermes no longer lists it: that closes it.
    let effects = resume(&mut g, &snapshot(json!([]), json!([]), 10), 120_500);
    assert!(names(&g).is_empty(), "{effects:?}");

    // The same request seen as it was asked has a deadline, kept when a
    // later reading lists it again.
    let mut live = model();
    feed(&mut live, &frame);
    assert_eq!(live.waiting().next().unwrap().deadline_ms, Some(20_000));
    resume(
        &mut live,
        &snapshot(json!([frame["params"]["payload"]]), json!([]), 10),
        5_000,
    );
    let pending = live.waiting().next().unwrap();
    assert_eq!(
        (pending.deadline_ms, pending.recheck_ms),
        (Some(20_000), None)
    );
}

#[test]
fn same_instance_reconnect_preserves_card_and_deadline() {
    let mut g = model();
    identified(&mut g, "instance-1");
    let mut frame = ask("kept", 1, 10);
    frame["params"]["payload"]["timeout_s"] = json!(20);
    feed(&mut g, &frame);
    let run = g.run();
    g.apply(Input::Lost(Failure::Closed(1006)), 100);
    identified(&mut g, "instance-1");
    assert_eq!(g.run(), run);
    assert_eq!(names(&g), ["kept"]);
    assert_eq!(g.next_deadline(), Some(20_000));
    assert!(feed(&mut g, &frame).is_empty());
}

#[test]
fn new_instance_identity_closes_existing_cards() {
    let mut g = model();
    identified(&mut g, "instance-1");
    feed(&mut g, &ask("old", 1, 10));
    identified(&mut g, "instance-2");
    assert!(names(&g).is_empty());
}

#[test]
fn approval_scopes_and_batch_multiselect_encode_exactly() {
    let session = model().session(&profile(), S);
    let approval = Approval {
        prompt: Prompt {
            id: "p".into(),
            generation: 4,
        },
        command: "synthetic".into(),
        description: "synthetic".into(),
        choices: vec![Choice::Once, Choice::Session, Choice::Always, Choice::Deny],
    };
    let asked = Pending {
        session: session.clone(),
        ask: Ask::Approval(approval),
        deadline_ms: None,
        recheck_ms: None,
    };
    for (kind, scope, word) in [
        ("allow", None, "once"),
        ("deny", None, "deny"),
        ("always", None, "always"),
        ("always", Some("session"), "session"),
    ] {
        let mut decision = json!({"kind": kind});
        if let Some(scope) = scope {
            decision["suggestion"] = json!({"type": "scope", "scope": scope});
        }
        let Some(Reply::Call(call)) = asked.reply(&decision, Scope::Host).unwrap() else {
            panic!("a call");
        };
        assert_eq!(
            call.frame(7),
            json!({"jsonrpc": "2.0", "id": 7, "method": "approval.respond", "params": {
                "session_id": S, "prompt_id": "p", "execution_generation": 4,
                "choice": word, "profile": "verify"}})
        );
    }
    let mut params = batch(json!({}))["params"].clone();
    params["questions"][1]["multi_select"] = json!(true);
    let ask = Ask::batch("srq-synthetic", "clarify", &params).unwrap();
    let set = Pending {
        session,
        ask,
        deadline_ms: None,
        recheck_ms: None,
    };
    let decision =
        json!({"kind": "answer", "answers": {"q1": {"selected": ["a (Recommended)", "b"]}}});
    assert_eq!(
        set.reply(&decision, Scope::Host).unwrap(),
        Some(Reply::Result(
            json!({"jsonrpc": "2.0", "id": "srq-synthetic",
            "result": {"answers": {"q1": "[\"a (Recommended)\",\"b\"]"}}})
        ))
    );
}

#[test]
fn the_sessions_are_listed_slowly_even_when_nothing_seems_to_happen() {
    let mut g = model();
    let list = |g: &mut Gateway, status: &str| {
        let result = json!({"sessions": [{"id": S, "status": status}]});
        g.apply(
            Input::Listed {
                run: g.run(),
                profile: &profile(),
                result: &result,
            },
            0,
        );
    };
    // Nothing known, or everything idle: still listed, every ten seconds.
    assert_eq!((BUSY_POLL_MS, IDLE_POLL_MS), (1_000, 10_000));
    assert_eq!(g.poll_ms(), IDLE_POLL_MS);
    list(&mut g, "idle");
    assert_eq!(g.poll_ms(), IDLE_POLL_MS);
    // A session that starts, works or waits is followed every second.
    for status in ["starting", "working", "waiting"] {
        list(&mut g, status);
        assert_eq!(g.poll_ms(), BUSY_POLL_MS, "{status}");
    }
    // So is a card that still shows on a session that says it is idle.
    list(&mut g, "idle");
    feed(&mut g, &ask("shown", 1, 1));
    assert_eq!(g.poll_ms(), BUSY_POLL_MS);
}
