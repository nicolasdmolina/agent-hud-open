//! The reproductions of both independent reviews of the first state layer
//! (`evidence/12/verifier-regressions.rs`, 12 tests, and
//! `verifier-2-new-regressions.rs`, 9 failing tests and 2 controls), under
//! their own names. Each sets up in a pretend Hermes what the review set
//! up by hand-ordered inputs, and asserts only what can be seen from
//! outside: which cards show and which effects come out. Where a review
//! delivered an older reading after a newer one, the older reading here is
//! a poll that was begun, and asked its questions, before the newer one.
//!
//! What a review asserted of the old model's insides has nothing left to
//! assert: its cursors (`cursor()`, in tests 1 and 21 and both controls),
//! its subscriptions (`subscription()`, 21), its deadlines (`deadline_ms`
//! in 9, `next_deadline()` in 10) and its runs (`run()`, a control). The
//! model keeps none of them. Each of those tests still asserts the card
//! behaviour the review was after.
use super::super::*;
use super::fake::{ask, identified, names, poll, Hermes};
use serde_json::{json, Value};

const S: &str = "synthetic-session";

fn profile() -> Profile {
    Profile::named("verify").unwrap()
}

/// A model that is ready, and a Hermes with nothing in it.
fn start() -> (Gateway, Hermes) {
    started(GatewayId::new(Machine::This, Profile::Default))
}

fn started(id: GatewayId) -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(id);
    identified(&mut gateway, "instance-1");
    (gateway, Hermes::default())
}

/// Pairs of gateways that can hold the same profile and session id: two
/// on this machine, this machine's and a far one's, two on one far
/// machine, and one on each of two far machines.
fn pairs() -> Vec<(GatewayId, GatewayId)> {
    let (mini, studio) = (super::far(super::INSTALL), super::far(super::OTHER_INSTALL));
    let id = |machine: &Machine, anchor: Profile| GatewayId::new(machine.clone(), anchor);
    vec![
        (
            id(&Machine::This, Profile::Default),
            id(&Machine::This, profile()),
        ),
        (
            id(&Machine::This, Profile::Default),
            id(&mini, Profile::Default),
        ),
        (id(&mini, Profile::Default), id(&mini, profile())),
        (id(&mini, Profile::Default), id(&studio, Profile::Default)),
    ]
}

fn payload(id: &str, turn: u64) -> Value {
    json!({"kind": "clarify", "prompt_id": id, "execution_generation": turn,
        "question": "Synthetic question", "choices": ["a (Recommended)", "b"], "multi_select": false})
}

fn asks(hermes: &mut Hermes, id: &str, turn: u64) {
    hermes.publish(&profile(), S, "clarify.request", turn, payload(id, turn));
}

fn settles(hermes: &mut Hermes, id: &str, turn: u64) {
    let body = json!({"prompt_id": id, "execution_generation": turn});
    hermes.publish(&profile(), S, "clarify.settled", turn, body);
}

fn batch(answers: Value) -> Value {
    json!({"id": "srq-synthetic", "method": "clarify", "params": {
        "session_id": S, "questions": [
            {"qid": "q0", "question": "Synthetic first", "choices": ["a", "b"], "multi_select": false},
            {"qid": "q1", "question": "Synthetic second", "choices": ["a", "b"], "multi_select": false}],
        "answers": answers}})
}

/// A poll begun now and asked now, whose answer reaches the model later.
fn taken(gateway: &mut Gateway, hermes: &mut Hermes) -> Poll {
    let mut poll = gateway.poll().expect("the gateway is ready");
    ask(&mut poll, hermes);
    poll
}

fn late(gateway: &mut Gateway, poll: Poll) -> Vec<Effect> {
    gateway.apply(Input::Polled(poll), 1_000)
}

fn at(gateway: &Gateway) -> SessionRef {
    gateway.session(&profile(), S).unwrap()
}

// ---- Round 1 ----

#[test]
fn stale_snapshot_must_not_close_newer_live_card_or_regress_cursor() {
    let (mut g, mut h) = start();
    h.stream(&profile(), S).running = true;
    // Read while nothing waited; it comes back after the request shows.
    let older = taken(&mut g, &mut h);
    asks(&mut h, "new", 2);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["new"]);
    let effects = late(&mut g, older);
    assert_eq!(names(&g), ["new"], "older reading closed a live card");
    assert_eq!(effects, []);
}

#[test]
fn settlement_seen_before_snapshot_must_prevent_stale_open() {
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    // Read while it waited; it comes back after the settlement was seen.
    let older = taken(&mut g, &mut h);
    settles(&mut h, "old", 1);
    assert_eq!(poll(&mut g, &mut h, 0), []);
    let effects = late(&mut g, older);
    assert!(
        names(&g).is_empty(),
        "settled request reopened: {effects:?}"
    );
    assert_eq!(effects, []);
}

#[test]
fn turn_end_must_not_erase_retirement_of_newer_generation() {
    let (mut g, mut h) = start();
    asks(&mut h, "new", 2);
    let older = taken(&mut g, &mut h);
    settles(&mut h, "new", 2);
    // An earlier turn's completion, published late.
    h.publish(
        &profile(),
        S,
        "message.complete",
        1,
        json!({"status": "complete"}),
    );
    h.stream(&profile(), S).status = Some("working".into());
    assert_eq!(poll(&mut g, &mut h, 0), []);
    let effects = late(&mut g, older);
    assert!(
        names(&g).is_empty(),
        "old completion let it reopen: {effects:?}"
    );
    // And however often it is read again, it stays over.
    assert_eq!(poll(&mut g, &mut h, 1), []);
}

#[test]
fn late_old_instance_snapshot_must_not_repopulate_restarted_gateway() {
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["old"]);
    let older = taken(&mut g, &mut h);
    identified(&mut g, "instance-2");
    assert!(names(&g).is_empty());
    let effects = late(&mut g, older);
    assert!(
        names(&g).is_empty(),
        "old instance reopened it: {effects:?}"
    );
    assert_eq!(effects, []);
}

#[test]
fn epoch_rollover_on_live_event_must_request_reconciliation() {
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["old"]);
    // The stream is replaced. `old` settled in the stream that is gone, so
    // no event can say so; the new stream holds only a new question.
    h.new_epoch(&profile(), S, "epoch-2");
    h.stream(&profile(), S).prompts.clear();
    asks(&mut h, "new", 2);
    let effects = poll(&mut g, &mut h, 100);
    assert_eq!(
        h.snapshots(),
        1,
        "no snapshot for a card the stream cannot explain"
    );
    assert_eq!(names(&g), ["new"], "{effects:?}");
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::Close { id, why: Closed::Gone, .. } if id.ends_with(":old"))
    ));
    // Had it still waited, the snapshot would have kept it.
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    h.new_epoch(&profile(), S, "epoch-2");
    asks(&mut h, "new", 2);
    poll(&mut g, &mut h, 100);
    assert_eq!(
        (h.snapshots(), names(&g)),
        (1, vec!["new".to_owned(), "old".to_owned()])
    );
}

#[test]
fn batch_reread_must_update_locks_and_emit_changed_card() {
    let (mut g, mut h) = start();
    h.stream(&profile(), S).running = true;
    h.stream(&profile(), S).open = vec![batch(json!({}))];
    poll(&mut g, &mut h, 0);
    h.stream(&profile(), S).open = vec![batch(json!({"q0": "a", "q1": null}))];
    let effects = poll(&mut g, &mut h, 100);
    let pending = g.waiting().next().unwrap().clone();
    let Ask::Batch(set) = &pending.ask else {
        panic!("a set of questions");
    };
    assert_eq!(set.questions[0].locked, Some(Locked::Answered("a".into())));
    assert_eq!(set.questions[1].locked, Some(Locked::Skipped));
    // The same request, shown as it is now: no second card.
    assert_eq!(effects, [Effect::Changed(pending)]);
    assert_eq!(g.waiting().count(), 1);
    // Read again unchanged, nothing is said.
    assert_eq!(poll(&mut g, &mut h, 200), []);
}

#[test]
fn batch_card_must_distinguish_locked_and_unanswered_questions() {
    let (g, _) = start();
    let card = |answers: Value| {
        let ask = Ask::batch("srq-synthetic", "clarify", &batch(answers)["params"]).unwrap();
        let (session, wait_ms) = (at(&g), None);
        Pending {
            session,
            ask,
            wait_ms,
        }
        .view(0, None)
    };
    let (unlocked, locked) = (card(json!({})), card(json!({"q0": "a", "q1": null})));
    assert_ne!(unlocked, locked);
    assert_eq!(unlocked["questions"][0]["lock"], Value::Null);
    assert_eq!(
        locked["questions"][0]["lock"],
        json!({"state": "answered", "answer": "a"})
    );
    assert_eq!(locked["questions"][1]["lock"], json!({"state": "skipped"}));
}

#[test]
fn old_completion_must_not_mark_new_turn_idle() {
    let (mut g, mut h) = start();
    h.publish(&profile(), S, "message.start", 2, json!({}));
    h.publish(
        &profile(),
        S,
        "message.complete",
        1,
        json!({"status": "complete"}),
    );
    // What a session is doing is what Hermes lists, never worked out from
    // the order of its events.
    h.stream(&profile(), S).status = Some("working".into());
    poll(&mut g, &mut h, 0);
    assert_eq!(g.statuses(), [(at(&g), Status::Working)]);
    // Nor does the old completion take the new turn's question.
    asks(&mut h, "new", 2);
    h.publish(
        &profile(),
        S,
        "message.complete",
        1,
        json!({"status": "complete"}),
    );
    poll(&mut g, &mut h, 1);
    assert_eq!(names(&g), ["new"]);
    assert_eq!(g.statuses(), [(at(&g), Status::Waiting)]);
}

#[test]
fn replayed_duration_must_not_be_treated_as_known_absolute_deadline() {
    let (mut g, mut h) = start();
    let mut body = payload("old", 1);
    body["timeout_s"] = json!(20);
    h.publish(&profile(), S, "clarify.request", 1, body);
    poll(&mut g, &mut h, 100_000);
    // The card says how long Hermes waits and claims no moment it ends.
    assert_eq!(g.waiting().next().unwrap().wait_ms, Some(20_000));
    // No time that passes closes it: only Hermes saying it is over does.
    assert_eq!(poll(&mut g, &mut h, 100_000 + 3_600_000), []);
    assert_eq!(names(&g), ["old"]);
    settles(&mut h, "old", 1);
    poll(&mut g, &mut h, 100_000 + 3_600_001);
    assert!(names(&g).is_empty());
}

#[test]
fn same_instance_reconnect_preserves_card_and_deadline() {
    let (mut g, mut h) = start();
    let mut body = payload("kept", 1);
    body["timeout_s"] = json!(20);
    h.publish(&profile(), S, "clarify.request", 1, body);
    poll(&mut g, &mut h, 0);
    g.apply(Input::Lost(Failure::Closed(1006)), 100);
    assert_eq!(names(&g), ["kept"]);
    assert!(
        g.poll().is_none(),
        "nothing is polled while the socket is gone"
    );
    assert_eq!(identified(&mut g, "instance-1").len(), 1);
    assert_eq!(names(&g), ["kept"]);
    assert_eq!(g.waiting().next().unwrap().wait_ms, Some(20_000));
    // Read again after the reconnect, it is the same card: nothing is said.
    assert_eq!(poll(&mut g, &mut h, 200), []);
}

#[test]
fn new_instance_identity_closes_existing_cards() {
    let (mut g, mut h) = start();
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    let effects = identified(&mut g, "instance-2");
    assert!(names(&g).is_empty());
    assert!(matches!(
        &effects[0],
        Effect::Close { why: Closed::GatewayRestarted, session, .. } if *session == at(&g)
    ));
    assert_eq!(g.statuses(), []);
}

#[test]
fn approval_scopes_and_batch_multiselect_encode_exactly() {
    let (g, _) = start();
    let session = at(&g);
    let approval = Approval {
        prompt: Prompt {
            id: "p".into(),
            generation: 4,
        },
        command: "synthetic".into(),
        description: "synthetic".into(),
        choices: vec![Choice::Once, Choice::Session, Choice::Always, Choice::Deny],
    };
    let (ask, wait_ms) = (Ask::Approval(approval), None);
    let p = Pending {
        session: session.clone(),
        ask,
        wait_ms,
    };
    for (kind, scope, word) in [
        ("allow", None, "once"),
        ("deny", None, "deny"),
        ("always", None, "always"),
        ("always", Some("session"), "session"),
    ] {
        let mut d = json!({ "kind": kind });
        if let Some(scope) = scope {
            d["suggestion"] = json!({"type": "scope", "scope": scope});
        }
        let Some(Reply::Call(call)) = p.reply(&d, Scope::Host).unwrap() else {
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
    let b = Pending {
        session,
        ask,
        wait_ms: None,
    };
    let d = json!({"kind": "answer", "answers": {"q1": {"selected": ["a (Recommended)", "b"]}}});
    assert_eq!(
        b.reply(&d, Scope::Host).unwrap(),
        Some(Reply::Result(
            json!({"jsonrpc": "2.0", "id": "srq-synthetic",
            "result": {"answers": {"q0": null, "q1": "[\"a (Recommended)\",\"b\"]"}}})
        ))
    );
}

// ---- Round 2 ----

#[test]
fn newer_empty_snapshot_must_fence_older_unseen_prompt() {
    let (mut g, mut h) = start();
    // It waits, its request event already dropped from the stream, so the
    // older reading finds it by a snapshot. Agent HUD never showed it.
    asks(&mut h, "unseen", 1);
    h.publish(&profile(), S, "session.info", 1, json!({}));
    h.drop_through(&profile(), S, 1);
    let older = taken(&mut g, &mut h);
    assert_eq!(h.snapshots(), 1);
    // It settles, and a newer reading shows that nothing waits.
    settles(&mut h, "unseen", 1);
    assert_eq!(poll(&mut g, &mut h, 100), []);
    let effects = late(&mut g, older);
    assert!(
        names(&g).is_empty(),
        "older reading resurrected it: {effects:?}"
    );
}

#[test]
fn a_live_frame_must_not_hide_unread_lower_sequence_request() {
    let (mut g, mut h) = start();
    // The question at 11 and an unrelated event at 12 come in one reading.
    for _ in 0..10 {
        h.publish(&profile(), S, "message.delta", 2, json!({}));
    }
    asks(&mut h, "still-waits", 2);
    h.publish(&profile(), S, "session.info", 2, json!({}));
    poll(&mut g, &mut h, 100);
    assert_eq!(names(&g), ["still-waits"]);
    // Whatever else the session publishes while it waits, it keeps showing.
    for _ in 0..40 {
        h.publish(&profile(), S, "session.info", 2, json!({}));
    }
    assert_eq!(poll(&mut g, &mut h, 200), []);
    assert_eq!(names(&g), ["still-waits"]);
}

#[test]
fn a_late_list_must_not_close_a_newer_live_session() {
    let (mut g, mut h) = start();
    // The sessions were listed before this one existed.
    let older = taken(&mut g, &mut h);
    asks(&mut h, "new", 2);
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["new"]);
    let effects = late(&mut g, older);
    assert_eq!(
        names(&g),
        ["new"],
        "late list closed a live card: {effects:?}"
    );
    assert_eq!(g.statuses(), [(at(&g), Status::Waiting)]);
}

#[test]
fn equal_cursor_batch_readings_must_not_erase_newer_locks() {
    let (mut g, mut h) = start();
    h.stream(&profile(), S).running = true;
    h.stream(&profile(), S).open = vec![batch(json!({}))];
    poll(&mut g, &mut h, 0);
    // Read before another viewer answered q0, at the same place in the
    // stream: locking an answer publishes no event.
    let older = taken(&mut g, &mut h);
    h.stream(&profile(), S).open = vec![batch(json!({"q0": "a"}))];
    poll(&mut g, &mut h, 100);
    let effects = late(&mut g, older);
    let Ask::Batch(set) = &g.waiting().next().unwrap().ask else {
        panic!("a set of questions");
    };
    assert_eq!(
        set.questions[0].locked,
        Some(Locked::Answered("a".into())),
        "late reading erased another viewer's answer: {effects:?}"
    );
    assert_eq!(effects, []);
}

#[test]
fn a_failed_recovery_must_be_requested_again_after_same_instance_reconnect() {
    let (mut g, mut h) = start();
    // A question survived a reset of the replay cache: it waits, and its
    // new stream is empty.
    asks(&mut h, "survivor", 1);
    h.new_epoch(&profile(), S, "epoch-2");
    // The snapshot fails before it returns.
    h.failing.push(Step::Snapshot(at(&g)));
    assert_eq!(poll(&mut g, &mut h, 0), []);
    assert_eq!(h.snapshots(), 1);
    // The socket drops and comes back to the same instance.
    g.apply(Input::Lost(Failure::Closed(1006)), 2);
    identified(&mut g, "instance-1");
    let effects = poll(&mut g, &mut h, 3);
    assert_eq!(h.snapshots(), 1, "the snapshot was not asked for again");
    assert_eq!(names(&g), ["survivor"], "{effects:?}");
    // Without any reconnect, the very next poll asks again as well.
    let (mut g, mut h) = start();
    asks(&mut h, "survivor", 1);
    h.new_epoch(&profile(), S, "epoch-2");
    h.failing.push(Step::Snapshot(at(&g)));
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["survivor".to_owned()]));
}

#[test]
fn a_retired_batch_must_not_suppress_new_epoch_prompt_recovery() {
    let (mut g, mut h) = start();
    h.stream(&profile(), S).open = vec![batch(json!({}))];
    h.stream(&profile(), S).running = true;
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["srq-synthetic"]);
    // The set ends and its turn with it; a new turn asks a question.
    h.stream(&profile(), S).open.clear();
    let cancel = json!({"id": "srq-synthetic", "method": "clarify", "reason": "resolved"});
    h.publish(&profile(), S, "request.cancel", 1, cancel);
    h.publish(
        &profile(),
        S,
        "message.complete",
        1,
        json!({"status": "complete"}),
    );
    h.publish(&profile(), S, "message.start", 2, json!({}));
    poll(&mut g, &mut h, 50);
    assert!(names(&g).is_empty());
    asks(&mut h, "later", 2);
    // The stream is replaced before the question was ever read.
    h.new_epoch(&profile(), S, "epoch-2");
    let effects = poll(&mut g, &mut h, 100);
    assert_eq!(
        h.snapshots(),
        1,
        "an old set of questions suppressed the snapshot"
    );
    assert_eq!(names(&g), ["later"], "{effects:?}");
}

#[test]
fn completed_batch_turns_must_not_accumulate_unbounded_terminal_ids() {
    let (mut g, mut h) = start();
    let mut size = 0;
    for turn in 1..=2000_u64 {
        let id = format!("srq-retired-{turn}");
        let mut set = batch(json!({}));
        set["id"] = json!(id);
        h.stream(&profile(), S).open = vec![set];
        h.stream(&profile(), S).running = true;
        poll(&mut g, &mut h, 0);
        assert_eq!(names(&g), std::slice::from_ref(&id));
        h.stream(&profile(), S).open.clear();
        let cancel = json!({"id": id, "method": "clarify", "reason": "resolved"});
        h.publish(&profile(), S, "request.cancel", turn, cancel);
        h.publish(
            &profile(),
            S,
            "message.complete",
            turn,
            json!({"status": "complete"}),
        );
        // Hermes keeps 512 events of a stream.
        let kept = h.stream(&profile(), S).latest.saturating_sub(512);
        h.drop_through(&profile(), S, kept);
        poll(&mut g, &mut h, 0);
        assert!(names(&g).is_empty());
        // Everything the model holds prints in its debug form, and after
        // the first turn it never grows.
        let debug = format!("{g:?}");
        assert!(
            !debug.contains("srq-retired-"),
            "turn {turn} left an id behind"
        );
        if turn == 1 {
            size = debug.len();
        }
        assert!(
            debug.len() <= size + 8,
            "turn {turn}: {} from {size}",
            debug.len()
        );
    }
}

#[test]
fn late_snapshot_must_not_recreate_a_session_removed_by_newer_list() {
    let (mut g, mut h) = start();
    asks(&mut h, "ended", 1);
    poll(&mut g, &mut h, 0);
    // Still in flight when the session ends.
    let older = taken(&mut g, &mut h);
    h.streams.clear();
    let effects = poll(&mut g, &mut h, 100);
    assert!(matches!(
        &effects[..],
        [Effect::Close {
            why: Closed::SessionEnded,
            ..
        }]
    ));
    assert!(g.statuses().is_empty());
    let effects = late(&mut g, older);
    assert!(
        names(&g).is_empty(),
        "old reply recreated the card: {effects:?}"
    );
    assert!(g.statuses().is_empty(), "old reply recreated the session");
}

#[test]
fn session_selectors_must_not_resolve_another_gateway_session() {
    // Each of a pair in turn is the one that holds the request.
    for (one, other) in pairs() {
        selectors_resolve_only_their_own(one.clone(), other.clone());
        selectors_resolve_only_their_own(other, one);
    }
}

fn selectors_resolve_only_their_own(holds: GatewayId, other: GatewayId) {
    let (mut local, mut h) = started(holds);
    let (mut remote, _) = started(other);
    asks(&mut h, "local-only", 1);
    poll(&mut local, &mut h, 0);
    let (own, foreign) = (at(&local), at(&remote));
    assert_eq!(local.status(&own), Some(Status::Waiting));
    assert_eq!(
        local.status(&foreign),
        None,
        "a remote session selected the local one"
    );
    assert_eq!(
        local
            .pending(&own.request_id("local-only"))
            .map(|p| &p.session),
        Some(&own)
    );
    assert_eq!(local.pending(&foreign.request_id("local-only")), None);
    assert_ne!(
        own.request_id("local-only"),
        foreign.request_id("local-only")
    );
    // The other gateway neither lists the session nor shows its card.
    assert_eq!(remote.status(&own), None);
    assert_eq!(remote.pending(&own.request_id("local-only")), None);
    assert!(remote.statuses().is_empty() && names(&remote).is_empty());
    // What another gateway's poll found is never this gateway's.
    let theirs = taken(&mut remote, &mut h);
    assert_eq!(late(&mut local, theirs), []);
    assert_eq!(names(&local), ["local-only"]);
    // An answer to the other gateway's request is not this one's.
    let (theirs, reply) = (
        foreign.request_id("local-only"),
        json!({"result": {"status": "resolved"}}),
    );
    let answered = Input::Responded {
        request: &theirs,
        reply: &reply,
    };
    assert_eq!(local.apply(answered, 2), []);
    assert_eq!(names(&local), ["local-only"]);
    // Every effect names the gateway its session is on.
    let effects = identified(&mut local, "instance-2");
    assert!(matches!(&effects[0], Effect::Close { session, .. } if *session == own));
}

// ---- Round 2's two controls, which passed before ----

#[test]
fn gateway_owned_state_and_restart_are_isolated() {
    for (one, other) in pairs() {
        let (mut a, mut b) = (Gateway::new(one), Gateway::new(other));
        let mut h = Hermes::default();
        asks(&mut h, "same", 1);
        for g in [&mut a, &mut b] {
            identified(g, "instance-1");
            poll(g, &mut h, 0);
        }
        assert_ne!(
            a.waiting().next().unwrap().id(),
            b.waiting().next().unwrap().id()
        );
        // Each lists the session as its own, and only its own.
        for (g, not) in [(&a, &b), (&b, &a)] {
            assert_eq!(g.statuses(), [(at(g), Status::Waiting)]);
            assert_eq!(g.status(&at(not)), None);
        }
        identified(&mut a, "instance-2");
        assert!(names(&a).is_empty());
        assert!(a.statuses().is_empty());
        assert_eq!(names(&b), ["same"]);
        assert_eq!(b.statuses(), [(at(&b), Status::Waiting)]);
        assert_eq!(poll(&mut b, &mut h, 1), []);
    }
}

#[test]
fn a_genuine_new_epoch_resets_cursor_and_keeps_current_run_reply() {
    let (mut g, mut h) = start();
    for _ in 0..99 {
        h.publish(&profile(), S, "message.delta", 1, json!({}));
    }
    asks(&mut h, "old", 1);
    poll(&mut g, &mut h, 0);
    g.apply(Input::Lost(Failure::Closed(1006)), 1);
    identified(&mut g, "instance-1");
    // A new stream that starts again at 1, with a new question in it.
    h.new_epoch(&profile(), S, "epoch-2");
    h.stream(&profile(), S).prompts.clear();
    asks(&mut h, "new", 2);
    poll(&mut g, &mut h, 2);
    assert_eq!(names(&g), ["new"]);
    // An answer sent on the socket from before the stream changed still lands.
    let request = g.waiting().next().unwrap().id();
    let reply = json!({"result": {"status": "resolved"}});
    let effects = g.apply(
        Input::Responded {
            request: &request,
            reply: &reply,
        },
        3,
    );
    assert!(names(&g).is_empty());
    assert_eq!(effects.last(), Some(&Effect::Poll));
}
