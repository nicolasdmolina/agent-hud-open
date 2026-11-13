//! How a card's life ends, and how the model finds what waits. The frames
//! here are written by hand in the recorded shapes, to reach the orders and
//! losses the recordings did not happen to hit.
use super::super::*;
use super::{fixture, local, spike};
use serde_json::{json, Value};

const SESSION: &str = "local-aaaa";
const EPOCH: &str = "epoch-1";

fn event(kind: &str, generation: u64, seq: u64, payload: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": "event", "params": {
        "type": kind, "session_id": SESSION, "execution_generation": generation,
        "payload": payload, "replay_epoch": EPOCH, "seq": seq,
    }})
}

fn question(prompt: &str, generation: u64, seq: u64, timeout: Option<u64>) -> Value {
    let mut payload = json!({"kind": "clarify", "prompt_id": prompt,
        "execution_generation": generation, "question": "Which one?",
        "choices": ["a", "b"], "multi_select": false});
    if let Some(timeout) = timeout {
        payload["timeout_s"] = json!(timeout);
    }
    event("clarify.request", generation, seq, payload)
}

fn batch(request: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": request, "method": "clarify", "params": {
        "session_id": SESSION,
        "questions": [{"qid": "q0", "question": "Which one?", "choices": ["a", "b"], "multi_select": false}],
    }})
}

fn complete(generation: u64, seq: u64, status: &str) -> Value {
    event(
        "message.complete",
        generation,
        seq,
        json!({"status": status}),
    )
}

fn feed(gateway: &mut Gateway, frame: &Value, now_ms: i64) -> Vec<Effect> {
    gateway.apply(
        Input::Frame {
            run: gateway.run(),
            profile: &spike(),
            frame,
        },
        now_ms,
    )
}

/// The cards closed by `effects`, with why.
fn closed(effects: &[Effect]) -> Vec<(&str, &Closed)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Close { id, why, .. } => Some((id.rsplit(':').next().unwrap(), why)),
            _ => None,
        })
        .collect()
}

fn opened(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Open(pending) => Some(pending.ask.key().to_owned()),
            _ => None,
        })
        .collect()
}

fn waiting(gateway: &Gateway) -> Vec<String> {
    gateway
        .waiting()
        .map(|pending| pending.ask.key().to_owned())
        .collect()
}

fn id(gateway: &Gateway, asked: &str) -> String {
    gateway.session(&spike(), SESSION).request_id(asked)
}

#[test]
fn a_turn_that_ended_takes_only_its_own_cards() {
    let mut gateway = local();
    feed(&mut gateway, &question("old", 1, 1, Some(20)), 0);
    feed(&mut gateway, &question("new", 2, 2, Some(20)), 0);
    // The completion of turn 1, read after turn 2 asked: the newer card stays.
    let effects = feed(&mut gateway, &complete(1, 3, "complete"), 0);
    assert_eq!(closed(&effects), [("old", &Closed::TurnEnded)]);
    assert_eq!(waiting(&gateway), ["new"]);
    let effects = feed(&mut gateway, &complete(2, 4, "interrupted"), 0);
    assert_eq!(closed(&effects), [("new", &Closed::Interrupted)]);
    // A completion that names no turn takes nothing.
    feed(&mut gateway, &question("third", 3, 5, None), 0);
    let mut unnamed = complete(3, 6, "complete");
    unnamed["params"]
        .as_object_mut()
        .unwrap()
        .remove("execution_generation");
    assert_eq!(feed(&mut gateway, &unnamed, 0), []);
    assert_eq!(waiting(&gateway), ["third"]);
}

#[test]
fn a_set_of_questions_ends_when_hermes_withdraws_it_whatever_the_reason() {
    for (reason, why) in [
        ("resolved", Closed::Settled),
        ("timeout", Closed::TimedOut),
        ("interrupted", Closed::Interrupted),
        ("shutdown", Closed::Withdrawn("shutdown".into())),
        ("session_closed", Closed::Withdrawn("session_closed".into())),
        (
            "a reason not written yet",
            Closed::Withdrawn("a reason not written yet".into()),
        ),
    ] {
        let mut gateway = local();
        assert_eq!(opened(&feed(&mut gateway, &batch("srq-1"), 0)), ["srq-1"]);
        // It names no turn, so a turn ending does not take it.
        assert_eq!(feed(&mut gateway, &complete(1, 1, "complete"), 0), []);
        let cancel = json!({"id": "srq-1", "method": "clarify", "reason": reason});
        let effects = feed(&mut gateway, &event("request.cancel", 1, 2, cancel), 0);
        assert_eq!(closed(&effects), [("srq-1", &why)], "{reason}");
    }
}

#[test]
fn a_card_runs_out_only_where_hermes_said_how_long_it_waits() {
    let mut gateway = local();
    feed(&mut gateway, &question("timed", 1, 1, Some(20)), 1_000);
    feed(&mut gateway, &question("untimed", 1, 2, None), 1_000);
    feed(&mut gateway, &batch("srq-1"), 1_000);
    let deadlines: Vec<Option<i64>> = gateway.waiting().map(|p| p.deadline_ms).collect();
    // In the order of their ids: the set, the timed question, the untimed one.
    assert_eq!(deadlines, [None, Some(21_000), None]);
    assert_eq!(gateway.next_deadline(), Some(21_000));
    assert_eq!(gateway.apply(Input::Tick, 20_999), []);
    assert_eq!(
        closed(&gateway.apply(Input::Tick, 21_000)),
        [("timed", &Closed::TimedOut)]
    );
    // No deadline known: no timer ever closes it.
    assert_eq!(gateway.apply(Input::Tick, i64::MAX), []);
    assert_eq!(gateway.next_deadline(), None);
    assert_eq!(waiting(&gateway), ["srq-1", "untimed"]);
    // Hermes settling the timed one afterwards changes nothing.
    let settled = json!({"prompt_id": "timed", "execution_generation": 1});
    assert_eq!(
        feed(
            &mut gateway,
            &event("clarify.settled", 1, 3, settled),
            30_000
        ),
        []
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
        let mut gateway = local();
        feed(&mut gateway, &question("p1", 1, 1, Some(20)), 0);
        let request = id(&gateway, "p1");
        gateway.apply(Input::Sent { request: &request }, 10);
        let effects = gateway.apply(
            Input::Responded {
                run: gateway.run(),
                request: &request,
                reply,
            },
            20,
        );
        match why {
            Some(why) => {
                assert_eq!(closed(&effects), [("p1", &why)], "{reply}");
                assert_eq!(waiting(&gateway), [] as [&str; 0]);
            }
            // Refused for another reason: the request still waits, so its
            // card comes back with the gateway's reason.
            None => {
                let pending = gateway.pending(&request).unwrap().clone();
                let why = refusal(reply);
                assert!(why.is_some());
                assert_eq!(effects, [Effect::Unanswered { pending, why }]);
            }
        }
    }
    assert!(Refusal::PermissionDenied.retryable() && !Refusal::InvalidParams.retryable());
}

fn identify_answer(instance: &str, port: u16) -> Value {
    json!({"runtime_protocol": 1, "state": "ready", "instance_id": instance, "authority_epoch": 2,
        "api_origin": format!("http://127.0.0.1:{port}"), "capabilities": [SESSION_AUTHORITY],
        "served_profiles": [{"profile_id": "/h", "home": "/h"}]})
}

#[test]
fn a_restart_closes_every_card_and_a_dropped_socket_closes_none() {
    let mut gateway = local();
    let ready = |gateway: &mut Gateway, instance: &str, port: u16| {
        let answer = identify_answer(instance, port);
        gateway.apply(
            Input::Identified {
                answer: &answer,
                home: "/h",
            },
            0,
        )
    };
    ready(&mut gateway, "one", 5000);
    feed(&mut gateway, &question("p1", 1, 1, Some(20)), 0);
    feed(&mut gateway, &batch("srq-1"), 0);
    let request = id(&gateway, "p1");
    gateway.apply(Input::Sent { request: &request }, 0);

    // The socket drops: both still wait in Hermes, with the deadline they
    // had. The answer on its way may be lost, so its card is shown again.
    let effects = gateway.apply(Input::Lost(Failure::Closed(1006)), 5);
    let pending = gateway.pending(&request).unwrap().clone();
    assert_eq!(
        effects,
        [
            Effect::Unanswered { pending, why: None },
            Effect::Connection(Connection::Failed(Failure::Closed(1006))),
        ]
    );
    assert_eq!(waiting(&gateway), ["p1", "srq-1"]);
    assert_eq!(gateway.next_deadline(), Some(20_000));

    // The same instance again: nothing was lost.
    let effects = ready(&mut gateway, "one", 5000);
    assert!(closed(&effects).is_empty());
    assert_eq!(waiting(&gateway).len(), 2);

    // Another instance is a restart, even on the same port.
    gateway.apply(Input::Lost(Failure::Closed(1012)), 6);
    let effects = ready(&mut gateway, "two", 5000);
    assert_eq!(
        closed(&effects),
        [
            ("p1", &Closed::GatewayRestarted),
            ("srq-1", &Closed::GatewayRestarted)
        ]
    );
    assert!(matches!(gateway.connection(), Connection::Ready(instance) if instance.id == "two"));
    assert_eq!(gateway.statuses(), []);
}

#[test]
fn a_gateway_waiting_to_restart_can_still_be_answered() {
    let mut answer = identify_answer("one", 5000);
    answer["state"] = json!("draining");
    assert_eq!(identify(&answer, "/h"), Identified::Wait("draining".into()));
    answer["drain_reason"] = json!("restart_wait");
    assert!(matches!(identify(&answer, "/h"), Identified::Ready(instance) if instance.draining));
    answer["state"] = json!("stopped");
    assert_eq!(
        identify(&answer, "/h"),
        Identified::Failed(Failure::Invalid("invalid_runtime_state"))
    );
    // Only this machine's own gateway is ever dialled.
    for origin in [
        "http://192.168.1.4:5000",
        "http://example.com:5000",
        "http://127.0.0.1",
        "ftp://127.0.0.1:5000",
        "http://127.0.0.1:5000/path",
    ] {
        let mut answer = identify_answer("one", 5000);
        answer["api_origin"] = json!(origin);
        assert_eq!(
            identify(&answer, "/h"),
            Identified::Failed(Failure::Invalid("invalid_api_origin")),
            "{origin}"
        );
    }
}

fn listed(rows: &[(&str, &str)]) -> Value {
    let sessions: Vec<Value> = rows
        .iter()
        .map(|(id, status)| json!({"id": id, "status": status}))
        .collect();
    json!({"sessions": sessions})
}

fn replay(events: &[Value], latest: u64, snapshot: bool, open_requests: Value) -> Value {
    let events: Vec<&Value> = events.iter().map(|event| &event["params"]).collect();
    json!({"events": events, "latest_seq": latest, "last_sequence": latest, "epoch": EPOCH,
        "replay_epoch": EPOCH, "truncated": snapshot, "snapshot_required": snapshot,
        "count": events.len(), "open_requests": open_requests})
}

#[test]
fn what_already_waits_is_found_from_the_start_of_the_stream() {
    let mut gateway = local();
    let at = gateway.session(&spike(), SESSION);
    let read = |cursor: Option<(&str, u64)>| Effect::Read {
        session: at.clone(),
        cursor: cursor.map(|(epoch, seq)| Cursor {
            epoch: epoch.into(),
            seq,
        }),
    };
    // A session that waits or works is read; an idle one is left alone. A
    // session waiting on a set of questions says `working`.
    let rows = listed(&[
        (SESSION, "waiting"),
        ("local-idle", "idle"),
        ("local-busy", "working"),
    ]);
    let effects = gateway.apply(
        Input::Listed {
            run: gateway.run(),
            profile: &spike(),
            result: &rows,
        },
        0,
    );
    let busy = Effect::Read {
        session: gateway.session(&spike(), "local-busy"),
        cursor: None,
    };
    assert_eq!(effects, [read(None), busy]);

    // The first answer only tells the epoch. The next read starts at 0 in
    // it, not at the latest event, which would skip the waiting request.
    let nothing = replay(&[], 11, true, json!([]));
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &nothing,
        },
        0,
    );
    assert_eq!(effects, [read(Some((EPOCH, 0)))]);

    let held = replay(&[question("p1", 1, 11, Some(20))], 11, false, json!([]));
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &held,
        },
        100,
    );
    assert_eq!(opened(&effects), ["p1"]);
    assert_eq!(
        gateway.cursor(&at),
        Some(&Cursor {
            epoch: EPOCH.into(),
            seq: 11
        })
    );
    assert_eq!(gateway.statuses()[0], (at.clone(), Status::Waiting));

    // The same events read again open nothing twice.
    assert_eq!(
        gateway.apply(
            Input::Replayed {
                run: gateway.run(),
                profile: &spike(),
                session: SESSION,
                result: &held
            },
            200
        ),
        []
    );

    // A card still showing is read even once the session says it is idle.
    let rows = listed(&[(SESSION, "idle")]);
    let effects = gateway.apply(
        Input::Listed {
            run: gateway.run(),
            profile: &spike(),
            result: &rows,
        },
        300,
    );
    assert_eq!(effects, [read(Some((EPOCH, 11)))]);
    let settled = event(
        "clarify.settled",
        1,
        12,
        json!({"prompt_id": "p1", "execution_generation": 1}),
    );
    let tail = replay(
        &[settled, complete(1, 13, "complete")],
        13,
        false,
        json!([]),
    );
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &tail,
        },
        400,
    );
    assert_eq!(closed(&effects), [("p1", &Closed::Settled)]);
    assert_eq!(gateway.statuses()[0].1, Status::Idle);
}

#[test]
fn a_request_asked_and_settled_within_one_reading_is_never_shown() {
    let mut gateway = local();
    let settled = event(
        "clarify.settled",
        1,
        2,
        json!({"prompt_id": "p1", "execution_generation": 1}),
    );
    let both = replay(
        &[question("p1", 1, 1, Some(20)), settled],
        2,
        false,
        json!([]),
    );
    gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &replay(&[], 2, true, json!([])),
        },
        0,
    );
    assert_eq!(
        gateway.apply(
            Input::Replayed {
                run: gateway.run(),
                profile: &spike(),
                session: SESSION,
                result: &both
            },
            0
        ),
        []
    );
    assert_eq!(waiting(&gateway), [] as [&str; 0]);
}

fn snapshot(prompts: Value, open_requests: Value, last: u64) -> Value {
    json!({"session_id": SESSION, "replay_epoch": EPOCH, "last_sequence": last,
        "subscription_id": "sub-1", "prompts": prompts, "open_requests": open_requests})
}

#[test]
fn a_stream_cut_short_is_made_whole_by_a_snapshot() {
    let mut gateway = local();
    let at = gateway.session(&spike(), SESSION);
    feed(&mut gateway, &question("gone", 1, 5, Some(20)), 0);
    // The stream no longer holds the range asked for: only a snapshot says
    // what waits, and taking one subscribes.
    let cut = replay(&[], 900, true, json!([]));
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &cut,
        },
        0,
    );
    assert_eq!(
        effects,
        [Effect::Resume {
            session: at.clone()
        }]
    );

    let asked = question("kept", 2, 0, Some(30))["params"]["payload"].clone();
    let entry = &batch("srq-9");
    let open = json!([{"id": "srq-9", "method": "clarify", "params": entry["params"]}]);
    let whole = snapshot(json!([asked]), open, 900);
    let effects = gateway.apply(
        Input::Resumed {
            run: gateway.run(),
            profile: &spike(),
            result: &whole,
        },
        1_000,
    );
    // What it does not list is over; what it lists is shown.
    assert_eq!(closed(&effects), [("gone", &Closed::Gone)]);
    assert_eq!(opened(&effects), ["kept", "srq-9"]);
    // Read, not seen as it was asked: how long it has waited is unknown.
    let kept = gateway.pending(&id(&gateway, "kept")).unwrap();
    assert_eq!((kept.deadline_ms, kept.recheck_ms), (None, Some(31_000)));
    assert_eq!(
        gateway.cursor(&at),
        Some(&Cursor {
            epoch: EPOCH.into(),
            seq: 900
        })
    );
    assert_eq!(gateway.subscription(&at), Some("sub-1"));
    // An event the snapshot already covers is not read again.
    assert_eq!(
        feed(&mut gateway, &question("early", 2, 900, None), 1_000),
        []
    );

    // A snapshot that lags behind does not bring back a card closed this turn.
    let request = id(&gateway, "kept");
    let reply =
        json!({"jsonrpc": "2.0", "id": 4, "result": {"status": "resolved", "prompt_id": "kept"}});
    gateway.apply(
        Input::Responded {
            run: gateway.run(),
            request: &request,
            reply: &reply,
        },
        1_100,
    );
    let effects = gateway.apply(
        Input::Resumed {
            run: gateway.run(),
            profile: &spike(),
            result: &whole,
        },
        1_200,
    );
    assert_eq!(effects, []);
    // The socket that subscribed is gone: so is the subscription.
    gateway.apply(Input::Lost(Failure::Closed(1000)), 1_300);
    assert_eq!(gateway.subscription(&at), None);

    // A new epoch with a card from the old one: a replay of the new stream
    // cannot settle it, so the model asks for a snapshot, not a replay.
    let mut other = replay(&[], 3, true, json!([]));
    other["replay_epoch"] = json!("epoch-2");
    other["epoch"] = json!("epoch-2");
    let mut fresh = local();
    feed(&mut fresh, &question("held", 1, 5, None), 0);
    let effects = fresh.apply(
        Input::Replayed {
            run: fresh.run(),
            profile: &spike(),
            session: SESSION,
            result: &other,
        },
        0,
    );
    assert_eq!(effects, [Effect::Resume { session: at }]);
}

#[test]
fn a_snapshot_is_asked_for_only_where_a_request_may_wait_unseen() {
    let read = |gateway: &mut Gateway, result: &Value| {
        gateway.apply(
            Input::Replayed {
                run: gateway.run(),
                profile: &spike(),
                session: SESSION,
                result,
            },
            0,
        )
    };
    let list = |gateway: &mut Gateway, status: &str| {
        gateway.apply(
            Input::Listed {
                run: gateway.run(),
                profile: &spike(),
                result: &listed(&[(SESSION, status)]),
            },
            0,
        );
    };
    let at = local().session(&spike(), SESSION);
    let resume = [Effect::Resume {
        session: at.clone(),
    }];
    let cut = replay(&[], 900, true, json!([]));

    // A session that only works, cut short: a long turn outran the stream.
    // Nothing can wait unseen, so the reading goes on from the latest
    // event and the session is not subscribed to.
    let mut working = local();
    list(&mut working, "working");
    read(&mut working, &cut);
    assert_eq!(read(&mut working, &cut), []);
    assert_eq!(
        working.cursor(&at),
        Some(&Cursor {
            epoch: EPOCH.into(),
            seq: 900
        })
    );

    // One that waits, cut short: what it waits on is past, so a snapshot.
    let mut waits = local();
    list(&mut waits, "waiting");
    read(&mut waits, &cut);
    assert_eq!(read(&mut waits, &cut), resume);

    // One that waits, read whole, with nothing in the stream to wait on:
    // a snapshot, asked once for that place in the stream.
    let mut silent = local();
    list(&mut silent, "waiting");
    read(&mut silent, &cut);
    let whole = replay(&[], 900, false, json!([]));
    assert_eq!(read(&mut silent, &whole), resume);
    assert_eq!(read(&mut silent, &whole), []);
    // The snapshot lists nothing either (a password prompt is not shown):
    // it is not asked for again until the stream moves.
    silent.apply(
        Input::Resumed {
            run: silent.run(),
            profile: &spike(),
            result: &snapshot(json!([]), json!([]), 900),
        },
        0,
    );
    assert_eq!(read(&mut silent, &whole), []);
    assert_eq!(
        read(&mut silent, &replay(&[], 901, false, json!([]))),
        resume
    );
}

#[test]
fn a_set_of_questions_follows_what_is_open_now() {
    let mut gateway = local();
    let entry = batch("srq-1");
    let open = json!([{"id": "srq-1", "method": "clarify", "params": entry["params"]}]);
    gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &replay(&[], 4, true, json!([])),
        },
        0,
    );
    // Not subscribed, the request frame never arrives: the reading lists it.
    let listing = replay(&[], 4, false, open.clone());
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &listing,
        },
        0,
    );
    assert_eq!(opened(&effects), ["srq-1"]);

    // Answered: nothing replies to the result frame. A reading right after
    // may still list the set; one long after means the answer was dropped.
    let request = id(&gateway, "srq-1");
    gateway.apply(Input::Sent { request: &request }, 1_000);
    assert_eq!(
        gateway.apply(
            Input::Replayed {
                run: gateway.run(),
                profile: &spike(),
                session: SESSION,
                result: &listing
            },
            2_000
        ),
        []
    );
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &listing,
        },
        6_000,
    );
    let pending = gateway.pending(&request).unwrap().clone();
    assert_eq!(effects, [Effect::Unanswered { pending, why: None }]);

    // No longer listed by a reading further on in the stream: it is over,
    // with no event needed to say so.
    let effects = gateway.apply(
        Input::Replayed {
            run: gateway.run(),
            profile: &spike(),
            session: SESSION,
            result: &replay(&[], 5, false, json!([])),
        },
        7_000,
    );
    assert_eq!(closed(&effects), [("srq-1", &Closed::Gone)]);
}

#[test]
fn a_session_that_left_the_live_list_is_over_with_its_requests() {
    let mut gateway = local();
    let work = Profile::named("work");
    feed(&mut gateway, &question("p1", 1, 1, None), 0);
    gateway.apply(
        Input::Frame {
            run: gateway.run(),
            profile: &work,
            frame: &question("p1", 1, 1, None),
        },
        0,
    );
    // One list covers one profile: the other profile's session is not touched.
    let effects = gateway.apply(
        Input::Listed {
            run: gateway.run(),
            profile: &spike(),
            result: &listed(&[]),
        },
        0,
    );
    assert_eq!(closed(&effects), [("p1", &Closed::SessionEnded)]);
    let left: Vec<String> = gateway
        .waiting()
        .map(|pending| pending.session.hud_id())
        .collect();
    assert_eq!(left, ["hermes:work:local-aaaa"]);
    assert_eq!(gateway.statuses().len(), 1);
}

#[test]
fn the_same_ids_on_two_gateways_or_profiles_are_different_requests() {
    let mut ids = std::collections::BTreeSet::new();
    for name in ["macbook", "mini"] {
        let mut gateway = Gateway::new(GatewayId::new(name));
        for profile in [Profile::Default, Profile::named("work")] {
            let effects = gateway.apply(
                Input::Frame {
                    run: gateway.run(),
                    profile: &profile,
                    frame: &question("p1", 1, 1, None),
                },
                0,
            );
            let [Effect::Open(pending)] = &effects[..] else {
                panic!("one card");
            };
            assert_eq!(pending.session.gateway.as_str(), name);
            assert_eq!(pending.session.profile, profile);
            ids.insert(pending.id());
        }
        assert_eq!(gateway.waiting().count(), 2);
    }
    let expected = [
        "hermes:macbook:default:local-aaaa:p1",
        "hermes:macbook:work:local-aaaa:p1",
        "hermes:mini:default:local-aaaa:p1",
        "hermes:mini:work:local-aaaa:p1",
    ];
    assert_eq!(ids.iter().map(String::as_str).collect::<Vec<_>>(), expected);
}

#[test]
fn a_change_of_sessions_is_a_hint_to_list_them() {
    let mut gateway = local();
    for kind in ["sessions.changed", "platforms.changed"] {
        let hint = json!({"jsonrpc": "2.0", "method": "event", "params": {"type": kind, "session_id": "", "payload": {}}});
        assert_eq!(feed(&mut gateway, &hint, 0), [Effect::List]);
    }
    // Recorded: `sessions.changed` does arrive on an unsubscribed socket.
    let doc = fixture("session-list");
    let seen = doc["clients"]["hud"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["frame"]["params"]["type"] == "sessions.changed")
        .count();
    assert!(seen > 0);
}
