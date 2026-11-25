//! A request answered, lost from sight and asked again sounds again
//! (Agent HUD Fork, ticket 74; the ticket 66 round 9 verifier's P4 note).
//! Codex Desktop asks for an approval, the app looks and it sounds, the
//! answer goes out and the owner confirms it. The channel then loses sight
//! of the thread's requests (the stream drops, the thread becomes unknown,
//! or a state's requests cannot be read), and the owner asks again with the
//! very same request before the app takes any look. That is a new ask, and
//! the person hears it. Before ticket 74 it went by the name the first one
//! sounded under, and nothing was heard. The real channel on its real
//! sockets, a stand-in owner and a whole core.
use super::*;
use agent_hud_core::{AgentHudCore, SourceRoot};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Gap {
    /// Sight is never lost: the owner streams the same state again.
    None,
    Drop,
    Unknown,
    Unreadable,
}

/// One look: the session's state, the cards shown, the `needs-you` ids.
fn look(core: &mut AgentHudCore) -> (String, usize, BTreeSet<String>) {
    let snapshot = core.request("request_snapshot", json!({})).unwrap();
    let rows = snapshot["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
    let cards = snapshot["approvals"].as_array().unwrap().len();
    let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = alerts.iter().filter(|alert| alert["kind"] == "needs-you");
    let ids = needs.filter_map(|alert| alert["id"].as_str().map(str::to_owned));
    let state = row["state"].as_str().unwrap().to_owned();
    (state, cards, ids.collect())
}
fn connection(listener: &UnixListener, within: Duration) -> Option<UnixStream> {
    let end = Instant::now() + within;
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(4)))
                    .unwrap();
                break Some(socket);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break None,
        }
    }
}
fn follow(listener: &UnixListener) -> UnixStream {
    let mut socket = connection(listener, Duration::from_secs(20)).expect("no stream");
    let init = read(&mut socket);
    assert_eq!(init["method"], "initialize");
    response(&mut socket, &init, json!({"clientId":"stream-client"}));
    let discovery = read(&mut socket);
    response(&mut socket, &discovery, json!({}));
    assert_eq!(read(&mut socket)["params"]["following"], true);
    socket
}
/// The answer the channel sends on a socket of its own, if it sends one.
fn answer(listener: &UnixListener, within: Duration) -> Option<(UnixStream, Value)> {
    let mut socket = connection(listener, within)?;
    let init = read(&mut socket);
    response(&mut socket, &init, json!({"clientId":"answer-client"}));
    let discovery = read(&mut socket);
    response(&mut socket, &discovery, json!({}));
    let decision = read(&mut socket);
    assert_eq!(decision["params"]["requestId"], 7);
    Some((socket, decision))
}

/// The distinct `needs-you` ids at the look after the request is asked
/// again, and at the last look, after the owner refused any second answer.
fn replay(automatic: bool, gap: Gap) -> (usize, usize) {
    let temp = tempfile::Builder::new().prefix("").tempdir().unwrap();
    let data = temp.path().join("w");
    let sessions = temp.path().join("sessions");
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
    let mut core = AgentHudCore::new(roots, temp.path().join("app"));
    for (key, value) in [("sound", true), ("autoApprove", automatic)] {
        let setting = json!({"key": key, "value": value});
        core.request("set_setting", setting).unwrap();
    }
    core.request("refresh", json!({})).unwrap();
    assert_eq!(look(&mut core).0, "running");
    let requests = core.requests();
    let now = crate::hooks::now_ms;
    let mut observer =
        CodexDesktopObserver::start(data, requests.clone(), Arc::new(|| {})).unwrap();
    observer.update_sessions(&[json!({"id":THREAD,"source":AGENT.id,"state":"running","navigationTarget":{"kind":"codex-thread","id":THREAD}})]);
    let mut wire = permission("browser-use", Value::Null);
    wire["params"].as_object_mut().unwrap().remove("turnId");
    let mut frame = snapshot(vec![wire]);
    frame["threadRuntimeStatus"] = json!({"type":"active","activeFlags":["waitingOnApproval"]});
    let id = pending::parse(&frame, THREAD, "owner").unwrap()[0]
        .id
        .clone();
    let broadcast = |socket: &mut UnixStream, change: Value| {
        send(
            socket,
            json!({"type":"broadcast","method":"thread-stream-state-changed","version":11,"sourceClientId":"owner","params":{"hostId":"local","conversationId":THREAD,"change":change}}),
        );
    };
    let flagged = || {
        let all = requests.native_waiting_sessions();
        all.get(AGENT.id).is_some_and(|s| s.contains(THREAD))
    };
    let confirmed = || {
        let rows = requests.deliveries(now());
        rows.iter()
            .any(|row| row["id"] == id && row["state"] == "confirmed")
    };
    let ok = json!({"method":pending::ELICITATION_METHOD,"result":{"ok":true}});

    let mut stream = follow(&listener);
    broadcast(
        &mut stream,
        json!({"type":"snapshot","revision":1,"conversationState":frame}),
    );
    wait(flagged);
    if !automatic {
        wait(|| requests.views(now()).len() == 1);
        // The card sounds, and the person allows it.
        assert_eq!(look(&mut core).2.len(), 1, "the card");
        requests
            .answer(&id, &json!({"kind":"allow"}), now())
            .unwrap();
    }
    let (mut first, decision) = answer(&listener, Duration::from_secs(6)).expect("an answer");
    if automatic {
        // On its way, not yet approved: it sounds.
        assert_eq!(look(&mut core).2.len(), 1, "on its way");
    }
    response(&mut first, &decision, ok);
    wait(confirmed);
    drop(first);
    // Sight is lost, and no look is taken until the request is asked again.
    match gap {
        Gap::None => {}
        Gap::Drop => {
            drop(stream);
            stream = follow(&listener);
        }
        Gap::Unknown => {
            broadcast(
                &mut stream,
                json!({"type":"patches","baseRevision":40,"revision":41,"patches":[]}),
            );
            wait(|| {
                !observer
                    .shared
                    .state
                    .lock()
                    .unwrap()
                    .known
                    .contains_key(THREAD)
            });
        }
        Gap::Unreadable => {
            let mut unreadable = frame.clone();
            unreadable.as_object_mut().unwrap().remove("requests");
            broadcast(
                &mut stream,
                json!({"type":"snapshot","revision":2,"conversationState":unreadable}),
            );
            wait(|| {
                let state = observer.shared.state.lock().unwrap();
                let known = state.known.get(THREAD);
                known.is_some_and(|known| known.state.get("requests").is_none())
            });
            // Reconcile publishes after it lets go of the state.
            thread::sleep(Duration::from_millis(150));
        }
    }
    broadcast(
        &mut stream,
        json!({"type":"snapshot","revision":50,"conversationState":frame}),
    );
    wait(|| {
        let state = observer.shared.state.lock().unwrap();
        let known = state.known.get(THREAD);
        known.is_some_and(|known| known.state["requests"].is_array())
    });
    wait(flagged);
    thread::sleep(Duration::from_millis(150));
    let second = answer(&listener, Duration::from_millis(2500));
    let again = look(&mut core).2.len();
    if let Some((mut socket, decision)) = second {
        let refused = json!({"type":"response","requestId":decision["requestId"],"handledByClientId":"owner","resultType":"error","error":"fixture-owner-refused"});
        send(&mut socket, refused);
        drop(socket);
        thread::sleep(Duration::from_millis(600));
    }
    let last = look(&mut core);
    assert_eq!(last.0, "waiting");
    observer.stop();
    (again, last.2.len())
}

#[test]
fn a_request_answered_by_hand_lost_from_sight_and_asked_again_sounds_again() {
    for gap in [Gap::Drop, Gap::Unknown, Gap::Unreadable] {
        assert_eq!(replay(false, gap), (2, 2), "{gap:?}");
    }
}

#[test]
fn a_request_allowed_automatically_lost_from_sight_and_asked_again_sounds_again() {
    for gap in [Gap::Drop, Gap::Unknown, Gap::Unreadable] {
        assert_eq!(replay(true, gap), (2, 2), "{gap:?}");
    }
}

/// The control: a request answered and confirmed that the owner keeps
/// listing, the channel never losing sight of it, is the same ask: it
/// sounded once and sounds no more.
#[test]
fn a_request_answered_and_still_listed_sounds_once() {
    for automatic in [false, true] {
        assert_eq!(replay(automatic, Gap::None), (1, 1), "{automatic}");
    }
}
