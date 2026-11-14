//! The independent verifier's tests from the first review of ticket 13,
//! kept as written (`evidence/13/verifier-adversarial.patch`) but for the
//! last: the contract changed, and an origin that is not plain http on
//! this machine is now unsupported and never dialled. Only the pretend
//! gateway receives these answers.
use super::{
    fake::{Fake, Hold},
    lab, until, Lab,
};
use serde_json::json;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

fn hold(gateway: &Fake, method: &'static str) -> Arc<Hold> {
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some((method, held.clone()));
    held
}

fn answer(lab: &Lab) -> String {
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    lab.requests
        .answer(
            &id,
            &json!({"kind":"answer", "answers": {
                "e1bc9d4fa6": {"selected":["red"]}
            }}),
            crate::hooks::now_ms(),
        )
        .unwrap();
    id
}

#[test]
fn verifier_stop_during_resume_must_cancel_answer_before_write() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    let held = hold(&gateway, "session.resume");
    answer(&lab);
    until("answer resume held", || {
        held.entered.load(Ordering::Acquire)
    });
    lab.observer.take();
    assert!(lab.cards().is_empty());
    held.released.store(true, Ordering::Release);
    until("answer visit detached", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    assert!(
        gateway.world().calls("clarify.respond").is_empty(),
        "answer was sent after Observer::stop returned"
    );
}

#[test]
fn verifier_settlement_during_resume_must_cancel_answer_before_write() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    let held = hold(&gateway, "session.resume");
    answer(&lab);
    until("answer resume held", || {
        held.entered.load(Ordering::Acquire)
    });
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("model closes settled card", || {
        lab.log().contains("why=Settled")
    });
    held.released.store(true, Ordering::Release);
    until("answer visit detached", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    assert!(
        gateway.world().calls("clarify.respond").is_empty(),
        "answer was sent for a card already settled by the polling connection"
    );
}

#[test]
fn verifier_late_failed_answer_must_not_reregister_settled_card() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    gateway.world().answers_with = Some("invalid_params");
    let held = hold(&gateway, "clarify.respond");
    answer(&lab);
    until("failed answer reply held", || {
        held.entered.load(Ordering::Acquire)
    });
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("model closes settled card", || {
        lab.log().contains("why=Settled")
    });
    held.released.store(true, Ordering::Release);
    until("answer visit detached", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    std::thread::sleep(Duration::from_millis(150));
    assert!(lab.cards().is_empty());
    assert!(!lab.log().contains("unanswered id="));
}

#[test]
fn verifier_stale_poll_after_same_instance_reconnect_must_be_dropped() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    let held = hold(&gateway, "session.events.since");
    lab.observer.as_ref().unwrap().nudge();
    until("poll reply held", || held.entered.load(Ordering::Acquire));
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    gateway.cut();
    held.released.store(true, Ordering::Release);
    until("reconnected", || gateway.world().sockets >= 2);
    until("settled card closed after reconnect", || {
        lab.waiting().is_empty()
    });
    assert!(lab.cards().is_empty());
    assert_eq!(lab.log().matches("open id=").count(), 1);
    assert_eq!(gateway.world().lists_at_once, 1);
}

#[test]
fn verifier_poll_read_before_successful_answer_cannot_reopen_card() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    let held = hold(&gateway, "nonempty-replay");
    lab.observer.as_ref().unwrap().nudge();
    until("poll holds old pending request", || {
        held.entered.load(Ordering::Acquire)
    });
    let id = answer(&lab);
    until("answer confirmed", || lab.log().contains("why=Answered"));
    held.released.store(true, Ordering::Release);
    until("new poll reads settlement", || {
        gateway.world().calls("session.events.since").len() >= 6
    });
    std::thread::sleep(Duration::from_millis(150));
    assert!(lab.cards().is_empty());
    assert_eq!(lab.log().matches("open id=").count(), 1);
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
}

#[test]
fn verifier_permission_retry_must_stop_if_card_settled_during_resume() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    gateway.world().answers_with = Some("permission_denied");
    let held = hold(&gateway, "clarify.respond");
    answer(&lab);
    until("first answer refusal held", || {
        held.entered.load(Ordering::Acquire)
    });
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("card settled", || lab.log().contains("why=Settled"));
    let answering_socket = gateway.world().calls("clarify.respond")[0].0;
    held.released.store(true, Ordering::Release);
    until("answer visit detached", || {
        gateway
            .world()
            .calls("session.detach")
            .iter()
            .any(|(socket, _)| *socket == answering_socket)
    });
    assert_eq!(
        gateway.world().calls("clarify.respond").len(),
        1,
        "a settled request was answered a second time during retry"
    );
}

#[test]
fn verifier_hint_on_short_socket_must_request_poll() {
    use super::super::{log, visit::Visit, Model, Shared};
    use agent_hud_agents::hermes::gateway::{
        identify_line, Connection, Gateway, GatewayId, Input, Machine, Profile,
    };
    use std::{
        collections::HashMap,
        sync::{atomic::AtomicBool, Condvar, Mutex},
    };
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let session_id = gateway.play("question-answered-elsewhere", Some("clarify.request"));
    gateway.world().hint_on_resume = true;
    let mut model = Gateway::new(GatewayId::new(Machine::This, Profile::Default));
    let identified = super::ask(&lab.home, &identify_line(), Duration::from_secs(2)).unwrap();
    model.apply(
        Input::Identified {
            answer: &identified,
            home: lab.home.to_str().unwrap(),
        },
        crate::hooks::now_ms(),
    );
    let Connection::Ready(instance) = model.connection().clone() else {
        panic!("not ready")
    };
    let session = model
        .session(&Profile::named("hud-spike").unwrap(), &session_id)
        .unwrap();
    let shared = Shared {
        route: lab.route.clone(),
        requests: lab.requests.clone(),
        wake: Arc::new(|| {}),
        log: log::Log::new(&lab.data),
        pace: super::quick(),
        active: AtomicBool::new(true),
        wanted: AtomicBool::new(false),
        model: Mutex::new(Model {
            gateway: model,
            waiters: HashMap::new(),
            opened: HashMap::new(),
            answered: HashMap::new(),
            taken: HashMap::new(),
            visits: HashMap::new(),
            visited: 0,
            stopped: false,
        }),
        stopping: (Mutex::new(()), Condvar::new()),
        reader: "local".into(),
        label: None,
        apart: Default::default(),
        asked: Mutex::new(Profile::Default),
        born: Mutex::default(),
        found: Mutex::default(),
        gone: AtomicBool::new(false),
        awaits: std::sync::atomic::AtomicU64::new(0),
        look: Arc::default(),
        ends: Arc::default(),
        shows: Default::default(),
        deaf: AtomicBool::new(false),
    };
    let (visit, _) = Visit::begin(&shared, &instance, &session, Duration::from_secs(2)).unwrap();
    visit.end();
    assert!(
        shared.is_wanted(),
        "sessions.changed received during resume was discarded"
    );
}

#[test]
fn verifier_https_origin_is_unsupported_and_never_dialled() {
    use agent_hud_agents::hermes::gateway::{identify, Identified, Unsupported};
    // Contract (coordinator, after the first review): phase 1 is the local
    // http gateway only, and phase 2 reaches a far gateway through its own
    // forwarded loopback http. An origin that is not plain http on this
    // machine is left to the hooks: no ticket is asked for, nothing dials.
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let ready = gateway.world().identify.clone().unwrap();
    let https = ready["result"]["api_origin"]
        .as_str()
        .unwrap()
        .replace("http://", "https://");
    let mut answer = ready.clone();
    answer["result"]["api_origin"] = json!(https);
    assert_eq!(
        identify(&answer, lab.home.to_str().unwrap()),
        Identified::Unsupported(Unsupported::Origin)
    );
    gateway.world().identify = Some(answer);
    lab.follow();
    let said = "connection unsupported reason=api_origin_not_local_http";
    until("it is found unsupported", || lab.log().contains(said));
    std::thread::sleep(Duration::from_millis(300));
    let world = gateway.world();
    assert!(world.granted.is_empty(), "a ticket was asked for");
    assert_eq!(world.sockets + world.turned_away, 0, "it was dialled");
    drop(world);
    // The same gateway on plain http is connected to.
    gateway.world().identify = Some(ready);
    until("plain http is connected to", || {
        gateway.world().sockets == 1
    });
}
