//! The independent verifier's two probes from the third review of ticket
//! 25 (`evidence/25/reverify3-late-stop.rs`), as written: a route that
//! lets the opening wait where it looks for the ticket's home, or where
//! the ticket's reply comes back, until the observer has stopped. Nothing
//! may be asked of the gateway or dialled after that.
use super::super::{
    control::Silent,
    route::{local::Local, Home, Route, Stop},
    Shared,
};
use super::{
    fake::{Fake, Hold},
    lab, quick, until, Observer,
};
use agent_hud_agents::hermes::gateway::{Failure, Instance};
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    net::TcpStream,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

struct Late {
    local: Local,
    gate: Arc<Hold>,
    before_ticket: bool,
    homes: AtomicUsize,
    owner: Mutex<Option<Weak<Shared>>>,
    installed: Condvar,
    events: Mutex<Vec<String>>,
}
impl Late {
    fn note(&self, method: &str) {
        let mut owner = self.owner.lock().unwrap();
        while owner.is_none() {
            owner = self.installed.wait(owner).unwrap();
        }
        let shared = owner.as_ref().unwrap().upgrade().unwrap();
        self.events
            .lock()
            .unwrap()
            .push(format!("{method} active={}", shared.active()));
    }
    fn hold(&self) {
        self.gate.entered.store(true, Ordering::Release);
        while !self.gate.released.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Route for Late {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        let call = self.homes.fetch_add(1, Ordering::AcqRel);
        if self.before_ticket && call == 1 {
            self.note("ticket-home-enter");
            self.hold();
        }
        self.note("homes");
        self.local.homes()
    }
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        let value: Value = serde_json::from_slice(line).unwrap();
        let verb = value["verb"].as_str().unwrap();
        self.note(verb);
        let result = self.local.exchange(home, line, timeout);
        if !self.before_ticket && verb == "session-ticket" {
            self.note("ticket-reply-held");
            self.hold();
        }
        result
    }
    fn named(&self, home: &Home) -> Cow<'_, str> {
        self.local.named(home)
    }
    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        self.note("prepare");
        self.local.prepare(instance, stop)
    }
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.note("dial");
        self.local.dial(instance)
    }
}
fn run(before_ticket: bool) {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Late {
        local: Local::at(&lab.home),
        gate: Arc::new(Hold::default()),
        before_ticket,
        homes: AtomicUsize::new(0),
        owner: Mutex::default(),
        installed: Condvar::new(),
        events: Mutex::default(),
    });
    let observer = Observer::start_on(
        route.clone(),
        &lab.data,
        lab.requests.clone(),
        Arc::new(|| {}),
        quick(),
    )
    .unwrap();
    let shared = observer.local();
    *route.owner.lock().unwrap() = Some(Arc::downgrade(&shared));
    route.installed.notify_all();
    until("opening held after prepare", || {
        route.gate.entered.load(Ordering::Acquire)
    });
    let tickets_at_stop = gateway.world().granted.len();
    let began = Instant::now();
    drop(observer);
    let stop_ms = began.elapsed().as_millis();
    assert!(!shared.active());
    route.note("stop-returned");
    route.gate.released.store(true, Ordering::Release);
    until("stopped connection worker returned", || {
        Arc::strong_count(&shared) == 1
    });
    let events = route.events.lock().unwrap().clone();
    let stopped = events
        .iter()
        .position(|e| e.starts_with("stop-returned"))
        .unwrap();
    let after = &events[stopped + 1..];
    let tickets_after = after
        .iter()
        .filter(|e| e.starts_with("session-ticket"))
        .count();
    let dials_after = after.iter().filter(|e| e.starts_with("dial")).count();
    let (tickets_final, sockets_final) = {
        let w = gateway.world();
        (w.granted.len(), w.sockets)
    };
    eprintln!("LATE STOP before_ticket={before_ticket} stop_ms={stop_ms} tickets_at_stop={tickets_at_stop} tickets_final={} sockets_final={} tickets_called_after_stop={tickets_after} dials_called_after_stop={dials_after}\nEVENTS {}", tickets_final, sockets_final, json!(events));
    assert_eq!(
        (tickets_after, dials_after),
        (0, 0),
        "opening continued after stop returned"
    );
}
#[test]
fn reverify3_stop_after_prepare_before_ticket_must_not_ticket_or_dial() {
    run(true)
}
#[test]
fn reverify3_stop_while_ticket_reply_waits_must_not_dial() {
    run(false)
}
