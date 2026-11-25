//! What is known of an automatic answer does not outlast the stream it
//! was known on (Agent HUD Fork, ticket 66, round 9; the round 8
//! verifier's probe). Codex Desktop asks for an approval, it is allowed
//! automatically and the owner confirms the answer. The stream drops.
//! While the HUD is not connected the request is settled and the same
//! owner asks again with the very same request, which makes the same id.
//! On the new stream that is a new wait: it sounds, it is answered anew,
//! and only that answer's confirmation makes it approved. Before round 9
//! core still held the first request's dismissal and confirmed approval,
//! so the second waited with no card, no answer and no sound. The real
//! channel on its real sockets, a stand-in owner and a whole core.
use super::*;
use std::collections::BTreeSet;

type Seen = (String, usize, BTreeSet<String>);

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

/// The next connection the channel makes to the owner's socket, if it
/// makes one within a few seconds.
fn connection(listener: &UnixListener) -> Option<UnixStream> {
    let end = Instant::now() + Duration::from_secs(6);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                let wait = Some(Duration::from_secs(4));
                socket.set_read_timeout(wait).unwrap();
                break Some(socket);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break None,
        }
    }
}
fn accept(listener: &UnixListener) -> UnixStream {
    connection(listener).expect("no fixture connection")
}

/// The channel's stream, taken up to where it follows the thread.
fn follow(listener: &UnixListener) -> UnixStream {
    let mut socket = accept(listener);
    let init = read(&mut socket);
    assert_eq!(init["method"], "initialize");
    response(&mut socket, &init, json!({"clientId":"stream-client"}));
    let discovery = read(&mut socket);
    assert_eq!(discovery["method"], "thread-owner-discovery");
    response(&mut socket, &discovery, json!({}));
    assert_eq!(read(&mut socket)["params"]["following"], true);
    socket
}

/// The answer the channel sends on a socket of its own: the socket, and
/// the decision it carries, which is an Allow of request 7. None where
/// the channel sends no answer.
fn answer(listener: &UnixListener) -> Option<(UnixStream, Value)> {
    let mut socket = connection(listener)?;
    let init = read(&mut socket);
    response(&mut socket, &init, json!({"clientId":"answer-client"}));
    let discovery = read(&mut socket);
    response(&mut socket, &discovery, json!({}));
    let decision = read(&mut socket);
    assert_eq!(decision["method"], pending::ELICITATION_METHOD);
    assert_eq!(decision["params"]["requestId"], 7);
    assert_eq!(decision["params"]["response"]["action"], "accept");
    Some((socket, decision))
}

/// One replay. With `looks_first`, the app looks at the first wait while
/// its answer is on its way and again once it is confirmed. Gives the
/// looks taken on the new stream: with the second answer on its way, and
/// after the owner confirmed it. The second is None where the channel
/// sent no second answer, as with the switch off.
fn replay(automatic: bool, looks_first: bool) -> (Seen, Option<Seen>) {
    use agent_hud_core::{AgentHudCore, SourceRoot};
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
    assert_eq!(look(&mut core), ("running".into(), 0, BTreeSet::new()));
    let requests = core.requests();
    let now = crate::hooks::now_ms;
    let mut observer =
        CodexDesktopObserver::start(data, requests.clone(), Arc::new(|| {})).unwrap();
    observer.update_sessions(&[json!({"id":THREAD,"source":AGENT.id,"state":"running","navigationTarget":{"kind":"codex-thread","id":THREAD}})]);
    // An approval for this once: allowing #7 does not answer the one that
    // comes after it, though the two are the same on the wire.
    let mut wire = permission("browser-use", Value::Null);
    wire["params"].as_object_mut().unwrap().remove("turnId");
    let mut frame = snapshot(vec![wire]);
    frame["threadRuntimeStatus"] = json!({"type":"active","activeFlags":["waitingOnApproval"]});
    let id = pending::parse(&frame, THREAD, "owner").unwrap()[0]
        .id
        .clone();
    let publish = |socket: &mut UnixStream, revision: u64| {
        send(
            socket,
            json!({"type":"broadcast","method":"thread-stream-state-changed","version":11,"sourceClientId":"owner","params":{"hostId":"local","conversationId":THREAD,"change":{"type":"snapshot","revision":revision,"conversationState":frame}}}),
        );
    };
    let waiting = |core: &mut AgentHudCore| {
        let start = Instant::now();
        loop {
            let seen = look(core);
            if seen.0 == "waiting" {
                break seen;
            }
            assert!(
                start.elapsed() < Duration::from_secs(4),
                "no wait published"
            );
            thread::sleep(Duration::from_millis(5));
        }
    };
    let confirmed = |id: &str| {
        let rows = requests.deliveries(now());
        let mine = |row: &Value| row["id"] == id && row["state"] == "confirmed";
        rows.iter().any(mine)
    };
    let confirmation = json!({"method":pending::ELICITATION_METHOD,"result":{"ok":true}});

    let mut stream = follow(&listener);
    publish(&mut stream, 1);
    if !automatic {
        wait(|| requests.views(now()).len() == 1);
        requests
            .answer(&id, &json!({"kind":"allow"}), now())
            .unwrap();
    }
    let (mut first, decision) = answer(&listener).expect("the first answer");
    let mut sounded = 0;
    if looks_first {
        // On its way: with the switch on it has no card and is not yet
        // approved, so it sounds. With it off its card was not looked at,
        // and the wait that is left of it sounds.
        sounded = 1;
        let seen = waiting(&mut core);
        assert_eq!((seen.1, seen.2.len()), (0, sounded), "the first wait");
    }
    response(&mut first, &decision, confirmation.clone());
    wait(|| confirmed(&id));
    drop(first);
    if looks_first {
        assert_eq!(look(&mut core).2.len(), sounded, "confirmed");
    }
    // The stream drops before the state that settles the request and the
    // one that asks again. The new stream brings only the last.
    drop(stream);
    let mut reopened = follow(&listener);
    assert_eq!(look(&mut core).0, "running", "the old wait is over");
    publish(&mut reopened, 3);
    let on_its_way = waiting(&mut core);
    assert_eq!(look(&mut core), on_its_way, "the same look twice");
    // With the switch on, the request on the new stream is answered anew,
    // and that answer is the one that can make it approved.
    let second = if automatic { answer(&listener) } else { None };
    let after = second.map(|(mut second, decision)| {
        assert!(requests.auto_approved(now()).is_empty(), "approved early");
        response(&mut second, &decision, confirmation);
        wait(|| !requests.auto_approved(now()).is_empty());
        look(&mut core)
    });
    observer.stop();
    (on_its_way, after)
}

/// The verifier's replay: the app never looked at the first wait. The
/// request on the new stream sounds, once, with the switch on and off.
/// With it on, it is also answered anew, and that answer's confirmation
/// adds no sound.
#[test]
fn a_request_asked_again_under_its_id_after_the_stream_dropped_sounds() {
    for automatic in [true, false] {
        let (on_its_way, after) = replay(automatic, false);
        let seen = |look: &Seen| (look.0.clone(), look.1, look.2.len());
        let about = format!("automatic: {automatic}");
        assert_eq!(seen(&on_its_way), ("waiting".into(), 0, 1), "{about}");
        assert_eq!(after.is_some(), automatic, "{about}: a second answer");
        if let Some(after) = after {
            assert_eq!(after, on_its_way, "{about}: confirmed, it sounded again");
        }
    }
}

/// The same where the app had looked at the first wait, which sounded
/// while its answer was on its way, and saw it end when the stream
/// dropped. The request on the new stream is another wait: a second
/// sound.
#[test]
fn a_request_asked_again_after_its_first_wait_was_seen_to_end_sounds_again() {
    for automatic in [true, false] {
        let (on_its_way, after) = replay(automatic, true);
        let about = format!("automatic: {automatic}");
        assert_eq!(on_its_way.2.len(), 2, "{about}");
        assert_eq!(after.is_some(), automatic, "{about}: a second answer");
        if let Some(after) = after {
            assert_eq!(after, on_its_way, "{about}: confirmed, it sounded again");
        }
    }
}
