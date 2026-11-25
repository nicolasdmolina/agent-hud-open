//! A receipt speaks for the answer it reports and for no other (Agent HUD
//! Fork, ticket 66, round 8; the round 7 verifier's probe). Codex Desktop
//! can ask again under the id it used before: the same owner and the same
//! request on the wire make the same id, also after the first was settled.
//! Approve automatically allows each, and each answer goes out on a socket
//! of its own. The first answer's confirmation, coming while the second is
//! still on its way, must confirm nothing of the second request: its wait
//! sounds, as its card did before the sync. The real channel, its real
//! answer workers, a stand-in owner and a whole core.
use super::*;
use std::collections::BTreeSet;

type Seen = (String, usize, BTreeSet<String>);

/// (the session's state, cards shown, the needs-you ids as the host plays
/// them: each once).
fn look(core: &mut agent_hud_core::AgentHudCore) -> Seen {
    let snapshot = core.request("request_snapshot", json!({})).unwrap();
    let rows = snapshot["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
    let cards = snapshot["approvals"].as_array().unwrap().len();
    let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = alerts.iter().filter(|alert| alert["kind"] == "needs-you");
    let ids = needs.filter_map(|alert| alert["id"].as_str().map(str::to_owned));
    (
        row["state"].as_str().unwrap().to_owned(),
        cards,
        ids.collect(),
    )
}

/// The owner's socket takes a connection within a few seconds.
fn accept(listener: &UnixListener, case: &str) -> UnixStream {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                let wait = Some(Duration::from_secs(4));
                socket.set_read_timeout(wait).unwrap();
                break socket;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("{case}: no fixture connection: {e}"),
        }
    }
}

/// One replay. `case` says how the second request relates to the first;
/// the two looks are taken after the first answer's confirmation, with the
/// second answer still on its way, and after the owner refused the second.
fn replay(case: &str, automatic: bool) -> (Seen, Seen) {
    use agent_hud_core::{AgentHudCore, SourceRoot};
    let scratch = tempfile::Builder::new().prefix("").tempdir().unwrap();
    let data = scratch.path().join("w");
    let sessions = scratch.path().join("sessions");
    fs::create_dir_all(data.join("ipc")).unwrap(); // plain directory
    fs::create_dir(&sessions).unwrap(); // plain directory
    let listener = UnixListener::bind(data.join("ipc/ipc.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let at = (chrono::Utc::now() - chrono::Duration::seconds(2)).to_rfc3339();
    let lines = [
        json!({"timestamp":at,"type":"session_meta","payload":{"id":THREAD,"timestamp":at,"source":"vscode","originator":"Codex Desktop","cwd":"/fixture"}}),
        json!({"timestamp":at,"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-fixture"}}),
    ];
    let log: String = lines.iter().map(|line| format!("{line}\n")).collect();
    fs::write(sessions.join(format!("rollout-{THREAD}.jsonl")), log).unwrap();
    let roots = vec![SourceRoot {
        path: sessions,
        provider: "codex",
    }];
    let mut core = AgentHudCore::new(roots, scratch.path().join("app"));
    for (key, value) in [("sound", true), ("autoApprove", automatic)] {
        let setting = json!({"key": key, "value": value});
        core.request("set_setting", setting).unwrap();
    }
    core.request("refresh", json!({})).unwrap();
    assert_eq!(look(&mut core), ("running".into(), 0, BTreeSet::new()));
    let requests = core.requests();
    let mut observer =
        CodexDesktopObserver::start(data, requests.clone(), Arc::new(|| {})).unwrap();
    observer.update_sessions(&[json!({"id":THREAD,"source":AGENT.id,"state":"running","navigationTarget":{"kind":"codex-thread","id":THREAD}})]);
    // The channel's own stream waits at its first word, so the frames are
    // handed to it directly and its answer workers are driven from here.
    let mut parked = accept(&listener, case);
    assert_eq!(read(&mut parked)["method"], "initialize");
    let start_reply = |number: i64| {
        observer.shared.answers();
        let mut socket = accept(&listener, case);
        let init = read(&mut socket);
        assert_eq!(init["method"], "initialize");
        response(&mut socket, &init, json!({"clientId":"answer-client"}));
        let discovery = read(&mut socket);
        assert_eq!(discovery["method"], "thread-owner-discovery");
        response(&mut socket, &discovery, json!({}));
        let decision = read(&mut socket);
        assert_eq!(decision["method"], pending::ELICITATION_METHOD);
        assert_eq!(decision["params"]["requestId"], number);
        assert_eq!(decision["params"]["response"]["action"], "accept");
        (socket, decision)
    };
    let state = |pending: Vec<Value>| {
        let mut frame = snapshot(pending);
        frame["threadRuntimeStatus"] = json!({"type":"active","activeFlags":["waitingOnApproval"]});
        frame
    };
    let mut first = permission("computer-use", json!(["session"]));
    first["id"] = json!(8);
    if case == "the same number, no turn named" {
        first["params"].as_object_mut().unwrap().remove("turnId");
    }
    let reuses_id = matches!(
        case,
        "the same request again" | "the same number, no turn named"
    );
    let id_of = |frame: &Value| {
        pending::parse(frame, THREAD, "owner").unwrap()[0]
            .id
            .clone()
    };
    let initial = state(vec![first.clone()]);
    let first_id = id_of(&initial);
    observer.shared.reconcile(THREAD, "owner", initial);
    let now = crate::hooks::now_ms;
    if !automatic {
        assert_eq!(requests.views(now()).len(), 1);
        let allow = json!({"kind":"allow"});
        requests.answer(&first_id, &allow, now()).unwrap();
    }
    let (mut old_socket, old_decision) = start_reply(8);

    let mut second = first.clone();
    if case == "the same number in a new turn" {
        second["params"]["turnId"] = json!("turn-replacement");
    } else if !reuses_id {
        second["id"] = json!(9);
    }
    let second_id = id_of(&state(vec![second.clone()]));
    assert_eq!(first_id == second_id, reuses_id, "{case}");
    let next = match case {
        "another beside it, listed after" => state(vec![first.clone(), second.clone()]),
        "another beside it, listed before" => state(vec![second.clone(), first.clone()]),
        _ => {
            let mut done = first.clone();
            done["completed"] = json!(true);
            observer
                .shared
                .reconcile(THREAD, "owner", state(vec![done]));
            state(vec![second.clone()])
        }
    };
    observer.shared.reconcile(THREAD, "owner", next);
    let later = if automatic {
        Some(start_reply(second["id"].as_i64().unwrap()))
    } else {
        assert_eq!(requests.views(now()).len(), 1);
        None
    };

    // The owner confirms the first answer, and only that one.
    let confirmation = json!({"method":pending::ELICITATION_METHOD,"result":{"ok":true}});
    response(&mut old_socket, &old_decision, confirmation);
    let received = |id: &str, delivery: &str| {
        let rows = requests.deliveries(now());
        let mine = |row: &Value| row["id"] == id && row["state"] == delivery;
        rows.iter().any(mine)
    };
    wait(|| received(&first_id, "confirmed"));
    let on_its_way = look(&mut core);
    let mut after = on_its_way.clone();
    if let Some((mut socket, decision)) = later {
        send(
            &mut socket,
            json!({"type":"response","requestId":decision["requestId"],"handledByClientId":"owner","resultType":"error","error":"fixture-owner-refused"}),
        );
        wait(|| received(&second_id, "failed"));
        after = look(&mut core);
    }
    observer.stop();
    (on_its_way, after)
}

/// Every case, with Approve automatically on and off. While the second
/// answer is on its way the second request has sounded, once, and the
/// owner's refusal adds nothing. With the switch off it shows its card and
/// sounds, as before the sync. Before round 8 the two cases that reuse the
/// id were silent with the switch on: the first answer's receipt had
/// confirmed the second request.
#[test]
fn a_receipt_for_an_earlier_answer_confirms_nothing_of_a_request_that_reuses_its_id() {
    let cases = [
        "the same request again",
        "the same number, no turn named",
        "the same number in a new turn",
        "another beside it, listed after",
        "another beside it, listed before",
    ];
    for case in cases {
        for automatic in [true, false] {
            let (on_its_way, after) = replay(case, automatic);
            let about = format!("{case}, automatic: {automatic}");
            let cards = usize::from(!automatic);
            let seen = |look: &Seen| (look.0.clone(), look.1, look.2.len());
            assert_eq!(seen(&on_its_way), ("waiting".into(), cards, 1), "{about}");
            assert_eq!(
                seen(&after),
                ("waiting".into(), cards, 1),
                "{about}, refused"
            );
            assert_eq!(after.2, on_its_way.2, "{about}: it sounded again");
        }
    }
}
