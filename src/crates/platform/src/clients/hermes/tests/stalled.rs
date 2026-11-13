//! A gateway that stops taking an answer in. The pretend gateway answers
//! the resume and then reads nothing more from that socket; the answer is
//! large enough that it cannot all fit in the kernel's buffers, so its
//! write waits. The verifier's probes (`verifier2.rs`) stop the observer
//! at that point; these close the card, and let the write run out of time.
use super::{
    fake::{Fake, Hold},
    lab, until, Lab,
};
use serde_json::json;
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

/// A card, and an answer to it far larger than a socket's buffers, sent to
/// a gateway that stops reading once it has answered the resume.
fn stalled_answer(lab: &mut Lab, gateway: &Fake) -> (String, Arc<Hold>) {
    lab.follow();
    lab.hears(gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let words = "x".repeat(8 * 1024 * 1024);
    let decision = json!({"kind": "answer", "answers": {"e1bc9d4fa6": {"custom": words}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the gateway stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    (id, stalled)
}

#[test]
fn stopping_cuts_an_answer_that_is_being_written_and_returns_at_once() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let (_, stalled) = stalled_answer(&mut lab, &gateway);
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    until("the answer is being written", || shared.writing());
    // The model is free while the write waits: polls go on.
    let polled = gateway.world().calls("session.active_list").len();
    until("polls go on", || {
        gateway.world().calls("session.active_list").len() > polled + 3
    });
    assert!(shared.writing());
    let began = Instant::now();
    lab.observer.take();
    let took = began.elapsed();
    assert!(took < Duration::from_secs(1), "stop took {took:?}");
    // The gateway reads again at once, while the write would still have
    // time left: what it finds is a cut socket, not the rest of an answer.
    stalled.released.store(true, Ordering::Release);
    until("the write is over", || !shared.writing());
    until("the gateway is done with the socket", || {
        gateway.world().visit_over()
    });
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty());
    // Nothing was written on that socket after the cut, a detach included.
    assert!(world.calls("session.detach").is_empty());
}

#[test]
fn a_card_that_closes_cuts_the_answer_being_written_to_it() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let (id, stalled) = stalled_answer(&mut lab, &gateway);
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    until("the answer is being written", || shared.writing());
    // Someone answers elsewhere: the poll sees it settled and closes the card.
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the card closes", || lab.log().contains("why=Settled"));
    // The gateway reads again at once: the write was cut with the card.
    stalled.released.store(true, Ordering::Release);
    until("the write is over", || !shared.writing());
    until("the gateway is done with the socket", || {
        gateway.world().visit_over()
    });
    assert!(gateway.world().calls("clarify.respond").is_empty());
    // The request is over: nothing comes back, and nothing is said of it.
    std::thread::sleep(Duration::from_millis(100));
    assert!(lab.cards().is_empty());
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
    assert!(!lab.log().contains("answer sent"), "{}", lab.log());
    assert!(shared.model().gateway.pending(&id).is_none());
}

#[test]
fn an_answer_the_gateway_does_not_take_gives_up_and_its_card_comes_back() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let began = Instant::now();
    let (id, stalled) = stalled_answer(&mut lab, &gateway);
    // Nobody stops anything: the write has its time (three seconds in the
    // tests, half a second in the app), all of it, and no more.
    let receipt = lab.delivery(&id);
    assert_eq!(receipt["state"], "failed");
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "{:?}",
        began.elapsed()
    );
    assert_eq!(lab.card()["id"], id.as_str());
    // What was left unsent is never sent later, with a detach or a close.
    stalled.released.store(true, Ordering::Release);
    until("the gateway is done with the socket", || {
        gateway.world().visit_over()
    });
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty());
    assert!(world.calls("session.detach").is_empty());
}

#[test]
fn what_a_failed_write_left_unsent_is_never_sent_with_a_later_frame() {
    use super::super::{control, socket::Socket};
    use agent_hud_agents::hermes::gateway::{
        granted, identify, identify_line, resume, ticket_line, GatewayId, Identified, Profile,
        Scope, SessionRef,
    };
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let id = gateway.play("question-answered-elsewhere", Some("clarify.request"));
    let wait = Duration::from_secs(5);
    let told = control::ask(&lab.home, &identify_line(), wait).unwrap();
    let Identified::Ready(instance) = identify(&told, lab.home.to_str().unwrap()) else {
        panic!("not ready")
    };
    let grant = control::ask(&lab.home, &ticket_line(&instance, Scope::Host), wait).unwrap();
    let mut socket = Socket::open(&instance, granted(&grant, &instance).unwrap()).unwrap();
    socket.write_within(Duration::from_millis(300));
    // Subscribed, and from then on the gateway reads nothing.
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let session = SessionRef {
        gateway: GatewayId::new("local"),
        profile: Profile::named("hud-spike"),
        id: id.clone(),
    };
    socket.send(&resume(&session, Scope::Host)).unwrap();
    until("the gateway stops reading", || {
        stalled.entered.load(Ordering::Acquire)
    });
    let answer = json!({"jsonrpc": "2.0", "id": 7, "method": "clarify.respond", "params": {
        "profile": "hud-spike", "session_id": id, "prompt_id": "e1bc9d4fa6",
        "execution_generation": 1, "answer": "x".repeat(8 * 1024 * 1024),
    }});
    let began = Instant::now();
    assert!(socket.write(&answer).is_err());
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    // The gateway reads again. Whatever is written now would carry the
    // rest of the answer with it: nothing is.
    stalled.released.store(true, Ordering::Release);
    let small = json!({"jsonrpc": "2.0", "id": 8, "method": "session.detach", "params": {
        "profile": "hud-spike", "session_id": id, "subscription_id": "x",
    }});
    assert!(socket.write(&small).is_err());
    socket.close();
    drop(socket);
    until("the gateway is done with the socket", || {
        gateway.world().visit_over()
    });
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty());
    assert!(world.calls("session.detach").is_empty());
}
