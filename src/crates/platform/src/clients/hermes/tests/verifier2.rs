//! The independent verifier's probes from the second review of ticket 13
//! (`evidence/13/verifier-2-extra.patch`), kept as written but for two
//! lines of `backpressure`, each of which waited for the defect itself:
//! "the answer writer holds the model lock" is now "an answer is being
//! written", and "the buffered answer arrives after stop" is now "the
//! gateway is done with the answer's socket". The look at the model lock
//! is taken twenty times, not once, since a poll holds it for a moment
//! now and then. Every assertion stands. All
//! connections use the pretend gateway in isolated homes.
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use serde_json::json;
use std::{
    sync::{atomic::Ordering, mpsc, Arc},
    time::{Duration, Instant},
};

struct Backpressure {
    locked: bool,
    polls_stalled: bool,
    stop_time: Duration,
    late_answers: usize,
}

fn backpressure() -> Backpressure {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let stalled = Arc::new(Hold::default());
    gateway.world().stall_after_resume = Some(stalled.clone());
    let shared = lab.observer.as_ref().unwrap().local();
    let decision = json!({"kind":"answer", "answers": {
        "e1bc9d4fa6": {"custom": "x".repeat(8 * 1024 * 1024)}
    }});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("answer peer stopped reading after resume", || {
        stalled.entered.load(Ordering::Acquire)
    });
    until("an answer is being written", || shared.writing());
    std::thread::sleep(Duration::from_millis(250));
    // Held at every look, as a write under the lock would hold it. One
    // look alone can meet a poll that has the lock for a moment.
    let locked = (0..20).all(|_| {
        let held = shared.model.try_lock().is_err();
        std::thread::sleep(Duration::from_millis(5));
        held
    });
    let lists_before = gateway.world().calls("session.active_list").len();
    std::thread::sleep(Duration::from_millis(500));
    let polls_stalled = lists_before == gateway.world().calls("session.active_list").len();
    eprintln!("Before stop: model_locked={locked} polls_stalled={polls_stalled}");
    let observer = lab.observer.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let stopped = std::thread::spawn(move || {
        let began = Instant::now();
        drop(observer);
        tx.send(began.elapsed()).unwrap();
    });
    until("stop requested", || !shared.active());
    let stop_time = rx.recv_timeout(Duration::from_secs(45)).unwrap();
    stopped.join().unwrap();
    eprintln!("Stop returned after {} ms", stop_time.as_millis());
    assert!(gateway.world().calls("clarify.respond").is_empty());
    stalled.released.store(true, Ordering::Release);
    until("the gateway is done with the answer's socket", || {
        gateway.world().visit_over()
    });
    let late_answers = gateway.world().calls("clarify.respond").len();
    eprintln!("BACKPRESSURE model_locked={locked} polls_stalled={polls_stalled} stop_ms={} answers_after_stop={late_answers}", stop_time.as_millis());
    Backpressure {
        locked,
        polls_stalled,
        stop_time,
        late_answers,
    }
}

#[test]
fn verifier2_stalled_write_must_not_hold_model_lock_or_block_stop() {
    let observed = backpressure();
    assert!(
        !observed.locked
            && !observed.polls_stalled
            && observed.stop_time < Duration::from_millis(250),
        "a stalled wire write held the model lock and blocked stop for {} ms",
        observed.stop_time.as_millis()
    );
}

#[test]
fn verifier2_failed_write_must_not_flush_answer_after_stop() {
    let observed = backpressure();
    assert_eq!(
        observed.late_answers, 0,
        "Visit::end flushed a failed answer after stop returned"
    );
}

#[test]
fn verifier2_non_http_or_non_loopback_origins_never_get_tickets() {
    use agent_hud_agents::hermes::gateway::{identify, Identified, Unsupported};
    for origin in [
        "https://127.0.0.1:5000",
        "https://[::1]:5000",
        "ftp://127.0.0.1:5000",
        "ws://127.0.0.1:5000",
        "http://192.0.2.1:5000",
        "http://example.com:5000",
        "http://localhost:5000",
    ] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        gateway.world().identify.as_mut().unwrap()["result"]["api_origin"] = json!(origin);
        let answer = gateway.world().identify.clone().unwrap();
        assert_eq!(
            identify(&answer, lab.home.to_str().unwrap()),
            Identified::Unsupported(Unsupported::Origin),
            "{origin}"
        );
        lab.follow();
        until("origin left unsupported", || {
            lab.log()
                .contains("connection unsupported reason=api_origin_not_local_http")
        });
        std::thread::sleep(Duration::from_millis(150));
        let world = gateway.world();
        assert!(world.granted.is_empty(), "ticket minted for {origin}");
        assert_eq!(world.sockets + world.turned_away, 0, "dialled {origin}");
        assert!(lab.cards().is_empty());
        assert!(lab.waiting().is_empty());
    }
}

#[test]
fn verifier2_initial_resume_must_not_write_after_currency_check_went_stale() {
    use super::super::visit::Visit;
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let shared = lab.observer.as_ref().unwrap().local();
    let instance = shared.instance_of(&id).unwrap();
    let pending = shared.model().gateway.pending(&id).unwrap().clone();
    let before = gateway.world().calls("session.resume").len();
    // Force the scheduling interleaving after current() releases its lock
    // but before Visit writes the resume. No answer frame is sent here.
    let still = || {
        let was_current = shared.current(&id, &instance);
        shared.stop();
        was_current
    };
    let visit = Visit::begin_while(
        &shared,
        &instance,
        &pending.session,
        Duration::from_secs(2),
        &still,
    );
    if let Some((visit, _)) = visit {
        visit.end();
    }
    let resumes_after_stop = gateway.world().calls("session.resume").len() - before;
    eprintln!("TOCTOU resumes_after_stop={resumes_after_stop}");
    assert_eq!(
        resumes_after_stop, 0,
        "resume was written after its currency check was invalidated by stop"
    );
}
