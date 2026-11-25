//! An automatic answer and what Codex Desktop made of it (Agent HUD Fork,
//! ticket 66, rounds 5 and 6; the round 4 verifier's probes). Approve
//! automatically allows the request and the channel sends the answer to
//! the thread's owner. The request is approved for sound only once the
//! owner confirms the answer. Where the owner refuses it, or goes away
//! before it confirms, the request still waits in Codex Desktop for the
//! person, and its wait sounds. So does a wait the app looks at while the
//! answer is still on its way. A stand-in owner on a Unix socket, the real
//! channel and a whole core.
use super::*;

/// (the session's state, cards shown, the needs-you ids as the host plays
/// them: each once).
fn seen(core: &mut agent_hud_core::AgentHudCore) -> (String, usize, usize) {
    let snapshot = core.request("request_snapshot", json!({})).unwrap();
    let rows = snapshot["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
    let state = row["state"].as_str().unwrap().to_owned();
    let cards = snapshot["approvals"].as_array().unwrap().len();
    let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = alerts.iter().filter(|item| item["kind"] == "needs-you");
    let ids: std::collections::HashSet<&str> =
        needs.filter_map(|item| item["id"].as_str()).collect();
    (state, cards, ids.len())
}

/// What the owner does with the automatic answer.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    Refuses,
    GoesAway,
    Confirms,
}

/// The look taken once the owner has answered, and the one taken before
/// (`looks_early`), while the automatic answer is still on its way.
fn an_automatic_answer(owner: Owner, looks_early: bool) -> (String, usize, usize) {
    use agent_hud_core::{AgentHudCore, SourceRoot};
    let scratch = tempfile::Builder::new().prefix("").tempdir().unwrap();
    let data = scratch.path().join("w");
    let sessions = scratch.path().join("sessions");
    fs::create_dir_all(data.join("ipc")).unwrap(); // plain directory
    fs::create_dir(&sessions).unwrap(); // plain directory
    let listener = UnixListener::bind(data.join("ipc/ipc.sock")).unwrap();
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
    for key in ["sound", "autoApprove"] {
        core.request("set_setting", json!({"key": key, "value": true}))
            .unwrap();
    }
    core.request("refresh", json!({})).unwrap();
    assert_eq!(seen(&mut core), ("running".into(), 0, 0));
    let requests = core.requests();

    let (finish, hold) = std::sync::mpsc::channel();
    let (reply_now, reply_ready) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let init = read(&mut stream);
        response(&mut stream, &init, json!({"clientId":"stream-client"}));
        let discovery = read(&mut stream);
        assert_eq!(discovery["method"], "thread-owner-discovery");
        response(&mut stream, &discovery, json!({}));
        assert_eq!(read(&mut stream)["params"]["following"], true);
        let mut current = snapshot(vec![permission("computer-use", json!(["session"]))]);
        current["threadRuntimeStatus"] =
            json!({"type":"active","activeFlags":["waitingOnApproval"]});
        send(
            &mut stream,
            json!({"type":"broadcast","method":"thread-stream-state-changed","version":11,"sourceClientId":"owner","params":{"hostId":"local","conversationId":THREAD,"change":{"type":"snapshot","revision":1,"conversationState":current}}}),
        );
        // The answer comes on a socket of its own.
        let (mut answer, _) = listener.accept().unwrap();
        answer
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let init = read(&mut answer);
        response(&mut answer, &init, json!({"clientId":"answer-client"}));
        let discovery = read(&mut answer);
        response(&mut answer, &discovery, json!({}));
        let decision = read(&mut answer);
        assert_eq!(decision["method"], pending::ELICITATION_METHOD);
        assert_eq!(decision["params"]["requestId"], 7);
        assert_eq!(decision["params"]["response"]["action"], "accept");
        reply_ready.recv_timeout(Duration::from_secs(4)).unwrap();
        match owner {
            Owner::GoesAway => drop(answer),
            Owner::Refuses => send(
                &mut answer,
                json!({"type":"response","requestId":decision["requestId"],"handledByClientId":"owner","resultType":"error","error":"fixture-owner-refused"}),
            ),
            Owner::Confirms => response(
                &mut answer,
                &decision,
                json!({"method":pending::ELICITATION_METHOD,"result":{"ok":true}}),
            ),
        }
        // The state stream stays as it is: the request is still listed.
        hold.recv_timeout(Duration::from_secs(4)).unwrap();
    });

    let mut observer =
        CodexDesktopObserver::start(data, requests.clone(), Arc::new(|| {})).unwrap();
    observer.update_sessions(&[json!({"id":THREAD,"source":AGENT.id,"state":"running","navigationTarget":{"kind":"codex-thread","id":THREAD}})]);
    let key = format!("codex:{THREAD}");
    wait(|| requests.native_wait_requests().contains_key(&key));
    let ids = requests.native_wait_requests()[&key].clone();
    assert_eq!(ids.len(), 1);
    // Allowed at once with no card, its wait named for it, and its answer
    // not yet confirmed: not approved, so a look now sounds.
    assert!(requests.auto_approved(crate::hooks::now_ms()).is_empty());
    if looks_early {
        assert_eq!(seen(&mut core), ("waiting".into(), 0, 1), "{owner:?}");
    }
    reply_now.send(()).unwrap();
    let receipt = match owner {
        Owner::Refuses => "failed",
        Owner::GoesAway => "uncertain",
        Owner::Confirms => "confirmed",
    };
    wait(|| {
        let receipts = requests.deliveries(crate::hooks::now_ms());
        let mine = |row: &Value| row["id"] == ids[0] && row["state"] == receipt;
        receipts.iter().any(mine)
    });
    // The owner still lists the request, so the wait still names it.
    assert_eq!(requests.native_wait_requests()[&key], ids, "{owner:?}");
    let after = seen(&mut core);
    assert_eq!(seen(&mut core), after, "{owner:?}: the next look differs");
    finish.send(()).unwrap();
    server.join().unwrap();
    observer.stop();
    after
}

#[test]
fn an_automatic_answer_the_owner_refuses_leaves_a_wait_that_sounds() {
    assert_eq!(
        an_automatic_answer(Owner::Refuses, false),
        ("waiting".into(), 0, 1)
    );
}

#[test]
fn an_automatic_answer_nobody_confirms_leaves_a_wait_that_sounds() {
    assert_eq!(
        an_automatic_answer(Owner::GoesAway, false),
        ("waiting".into(), 0, 1)
    );
}

/// The control: an answer the owner confirms before the app looks is
/// silent while Codex Desktop goes on listing the request for a moment.
#[test]
fn an_automatic_answer_the_owner_confirms_stays_silent() {
    assert_eq!(
        an_automatic_answer(Owner::Confirms, false),
        ("waiting".into(), 0, 0)
    );
}

/// Round 6. A look that lands while the answer is on its way sounds, as
/// the request's card did before the sync. Whatever the owner then says,
/// that is the one sound: a confirmation takes nothing back and adds
/// nothing, and a failure is the wait already told.
#[test]
fn a_look_while_the_automatic_answer_is_on_its_way_sounds_once() {
    for owner in [Owner::Confirms, Owner::Refuses, Owner::GoesAway] {
        assert_eq!(
            an_automatic_answer(owner, true),
            ("waiting".into(), 0, 1),
            "{owner:?}"
        );
    }
}
