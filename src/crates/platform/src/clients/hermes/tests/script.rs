//! One scripted run against the pretend gateway, and the log it leaves,
//! line for line: a home that is not there, one that is not private, a
//! question settled elsewhere, an approval answered here, a socket that
//! drops, a gateway that comes back as another instance, a build without
//! the live connection, a stop. Every step waits for what it caused, and
//! where a poll could land between two lines the poll is held, so the
//! order is the same every time. A change that is meant to change nothing
//! leaves this log as it is.
use super::{
    fake::{fixture, Fake, Hold},
    lab, until,
};
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{atomic::Ordering, Arc},
};

const FIRST: &str = "aaaaaaaa-first";
const SECOND: &str = "bbbbbbbb-second";

/// The session of each recording played, and the request it asks.
const ELSEWHERE: (&str, &str) = (
    "local-ab9cadb3b3c5d0dc9dc693d4eaa1e623a6bdc601bfd0912e6af6d1b5a096dd22",
    "e1bc9d4fa6",
);
const ONCE: (&str, &str) = (
    "local-e36d3be55be794058a8299d205c02a11513f18e745c8e6320c9f24cb02d2b709",
    "5c1c13ce8c354c329a25dafa08886e9f",
);
const HERE: (&str, &str) = (
    "local-f200386683510182cd45c8e38361a3bfecfa23934bacf51c394ec5b828f7ee9b",
    "715a2db63d",
);
const RESTART: (&str, &str) = (
    "local-cc1ed54c9a8de1d54405bb830a5397f50fae19b31713f6a656d01a6299e89fec",
    "d4dcbc0fc5504f99a72d973288b3554a",
);

/// The log of the run, without the time each line was written and
/// without "started" (see the end of the test).
fn expected() -> Vec<String> {
    let id =
        |(session, request): (&str, &str)| format!("hermes:local:hud-spike:{session}:{request}");
    let open = |of: (&str, &str), kind: &str| {
        let session = of.0;
        format!(
            "open id={} kind={kind} session=hermes:hud-spike:{session}",
            id(of)
        )
    };
    let close = |of, why: &str| format!("close id={} why={why}", id(of));
    let ready =
        |instance: &str| format!("connection ready instance={instance} profiles=2 draining=false");
    let lost = "connection failed reason=Closed(1006)".to_owned();
    let connecting = "connection connecting".to_owned();
    vec![
        connecting.clone(),
        "connection failed reason=Unreachable".to_owned(),
        "connection failed reason=Refused(\"the control home is not private\")".to_owned(),
        ready("aaaaaaaa"),
        open(ELSEWHERE, "question"),
        close(ELSEWHERE, "Settled"),
        open(ONCE, "approval"),
        format!("answer sent id={}", id(ONCE)),
        close(ONCE, "Answered"),
        open(HERE, "question"),
        lost.clone(),
        connecting.clone(),
        ready("aaaaaaaa"),
        close(HERE, "Settled"),
        open(RESTART, "approval"),
        lost.clone(),
        connecting.clone(),
        close(RESTART, "GatewayRestarted"),
        ready("bbbbbbbb"),
        lost,
        connecting,
        "connection unsupported reason=runtime_protocol".to_owned(),
        "stopped".to_owned(),
    ]
}

fn mode(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
}

/// Holds the gateway's next reply to `method` until released.
fn hold(gateway: &Fake, method: &'static str) -> Arc<Hold> {
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some((method, held.clone()));
    until("the gateway holds its reply", || {
        held.entered.load(Ordering::Acquire)
    });
    held
}

/// Cuts every socket of the gateway and waits until polls run again, on
/// a socket opened since.
fn cut(gateway: &Fake) {
    let before = gateway.world().sockets;
    gateway.cut();
    until("polls run on a new socket", || {
        let lists = gateway.world().calls("session.active_list");
        lists.iter().any(|(socket, _)| *socket > before)
    });
}

/// What the connection logged, a line each, without its time. The sums
/// of what the polls cost are left out: how many ran is a matter of time.
fn logged(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(_, said)| said.to_owned())
        .filter(|said| !said.starts_with("polls n=") && !said.starts_with("slow poll"))
        .collect()
}

#[test]
fn a_scripted_run_leaves_the_same_log_line_for_line() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, FIRST);
    let said = |lab: &super::Lab, what: &str, times: usize| {
        let seen = |lab: &super::Lab| {
            logged(&lab.log())
                .iter()
                .filter(|line| line.contains(what))
                .count()
        };
        until(what, || seen(lab) >= times);
    };

    // The home is not there: no gateway to ask.
    let parked = lab.home.with_extension("parked");
    std::fs::rename(&lab.home, &parked).unwrap();
    lab.follow();
    said(&lab, "reason=Unreachable", 1);
    // It is there, and others may write it: nothing in it is trusted.
    mode(&parked, 0o770);
    std::fs::rename(&parked, &lab.home).unwrap();
    said(&lab, "is not private", 1);
    mode(&lab.home, 0o700);
    said(&lab, "connection ready", 1);

    // A question, which the session's own client answers.
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the question's card goes", || lab.cards().is_empty());

    // An approval, answered here. No poll ends between the answer and
    // Hermes's reply, so the reply is what closes the card.
    lab.hears(&gateway, "approval-answered-once", "approval.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let held = hold(&gateway, "session.active_list");
    lab.requests
        .answer(&id, &json!({"kind": "allow"}), crate::hooks::now_ms())
        .unwrap();
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    held.released.store(true, Ordering::Release);

    // A socket that drops under a question: the card stays.
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    lab.card();
    cut(&gateway);
    said(&lab, "connection ready", 2);
    assert_eq!(lab.cards().len(), 1);
    gateway.play("question-answered-here", Some("clarify.settled"));
    until("the second question's card goes", || lab.cards().is_empty());

    // The gateway comes back as another instance with nothing waiting:
    // what the first one asked did not survive it.
    lab.hears(&gateway, "gateway-restart-hard", "approval.request");
    lab.card();
    let held = hold(&gateway, "session.active_list");
    {
        let mut world = gateway.world();
        world.identify.as_mut().unwrap()["result"]["instance_id"] = json!(SECOND);
        world.instance = SECOND.to_owned();
        world.streams.clear();
    }
    let before = gateway.world().sockets;
    gateway.cut();
    held.released.store(true, Ordering::Release);
    until("the new instance is polled", || {
        let lists = gateway.world().calls("session.active_list");
        lists.iter().any(|(socket, _)| *socket > before)
    });
    said(&lab, "connection ready", 3);
    assert!(lab.cards().is_empty());

    // A build without the live connection takes its place.
    let old = fixture("gateway-self-description-old-build.SYNTHETIC");
    let cases = old["cases"].as_array().unwrap();
    let case = cases
        .iter()
        .find(|case| case["verdict"]["reason_code"] == "runtime_protocol")
        .unwrap();
    let mut told = case["identify_result"].clone();
    let home = lab.home.to_string_lossy().into_owned();
    if told.get("served_profiles").is_some() {
        told["served_profiles"] = json!([{"profile_id": home, "home": home}]);
    }
    gateway.world().identify = Some(json!({"ok": true, "protocol": 1, "id": 1, "result": told}));
    gateway.cut();
    said(&lab, "connection unsupported", 1);

    lab.observer.take();
    said(&lab, "stopped", 1);

    let mut log = logged(&lab.log());
    // "started" is written by whoever starts the observer, while the
    // connection's own thread writes its first two lines: which comes
    // first is not decided. It is taken out of its place and looked at
    // alone; every other line is in the order it happened.
    let started = log.iter().position(|line| line == "started");
    assert!(started.is_some_and(|at| at < 3), "{log:?}");
    log.remove(started.unwrap());
    eprintln!("--- scripted log ---\n{}\n--- end ---", log.join("\n"));
    assert_eq!(log, expected());
}
