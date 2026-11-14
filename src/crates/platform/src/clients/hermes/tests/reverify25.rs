//! The independent verifier's three probes from the second review of
//! ticket 25 (`evidence/25/reverify2-extra.rs`): a route that makes a
//! real forward in `prepare` and dials through it, one whose preparation
//! fails, and one whose preparation is still under way when the observer
//! stops. They are as written but for what the review asked for: `prepare`
//! now hears of a stop (`Stop`), so the probe's route waits on that while
//! its gate holds it, and ends its preparation, making nothing, once the
//! observer has stopped. The assertions stand, and the last test has three
//! more: no forward was made, and nothing was asked of the gateway and
//! nothing dialled after the preparation began.
//!
//! One wait in the first test moved, after the fourth review: it waited
//! for every ticket granted to have become a socket after dropping the
//! observer. An opening that a stop overtakes drops its ticket and opens
//! nothing, so that count need not catch up once the observer is gone,
//! and the test failed 2 runs in 30. It now waits while the observer
//! still runs, and drops it after.
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
    io,
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

struct Probe {
    local: Local,
    events: Mutex<Vec<String>>,
    owner: Mutex<Option<Weak<Shared>>>,
    installed: Condvar,
    gate: Option<Arc<Hold>>,
    prepare_done: AtomicBool,
    fail: bool,
    forward: Mutex<Option<(String, SocketAddr)>>,
    forwards: AtomicUsize,
    shutdown: Arc<AtomicBool>,
}
impl Probe {
    fn at(home: &std::path::Path, gate: Option<Arc<Hold>>, fail: bool) -> Self {
        Self {
            local: Local::at(home),
            events: Mutex::default(),
            owner: Mutex::default(),
            installed: Condvar::new(),
            gate,
            prepare_done: AtomicBool::new(false),
            fail,
            forward: Mutex::default(),
            forwards: AtomicUsize::new(0),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
    fn install(&self, shared: &Arc<Shared>) {
        *self.owner.lock().unwrap() = Some(Arc::downgrade(shared));
        self.installed.notify_all();
    }
    fn note(&self, method: &str) {
        let mut owner = self.owner.lock().unwrap();
        while owner.is_none() {
            owner = self.installed.wait(owner).unwrap();
        }
        let shared = owner.as_ref().unwrap().upgrade().unwrap();
        let held = (0..20).all(|_| {
            let held = shared.model.try_lock().is_err();
            if held {
                thread::sleep(Duration::from_millis(1));
            }
            held
        });
        let event = format!("{method} active={} model_locked={held}", shared.active());
        self.events.lock().unwrap().push(event);
        assert!(!held, "route method {method} called under model lock");
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}
impl Route for Probe {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.note("homes");
        self.local.homes()
    }
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        let value: Value = serde_json::from_slice(line).unwrap();
        self.note(value["verb"].as_str().unwrap());
        self.local.exchange(home, line, timeout)
    }
    fn named(&self, home: &Home) -> Cow<'_, str> {
        self.note("named");
        self.local.named(home)
    }
    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        self.note("prepare-enter");
        if let Some(gate) = &self.gate {
            gate.entered.store(true, Ordering::Release);
            while !gate.released.load(Ordering::Acquire) {
                if stop.wait(Duration::from_millis(5)) {
                    self.prepare_done.store(true, Ordering::Release);
                    self.note("prepare-cancelled");
                    return Err(Failure::Unreachable);
                }
            }
        }
        if self.fail {
            self.prepare_done.store(true, Ordering::Release);
            self.note("prepare-failed");
            return Err(Failure::Unreachable);
        }
        let mut forward = self.forward.lock().unwrap();
        if forward.is_none() {
            let remote: SocketAddr = instance
                .origin
                .strip_prefix("http://")
                .unwrap()
                .parse()
                .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            assert_ne!(address, remote);
            listener.set_nonblocking(true).unwrap();
            let shutdown = self.shutdown.clone();
            thread::spawn(move || {
                while !shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut incoming, _)) => {
                            incoming.set_nonblocking(false).unwrap();
                            thread::spawn(move || {
                                let mut outgoing = TcpStream::connect(remote).unwrap();
                                let mut incoming_copy = incoming.try_clone().unwrap();
                                let mut outgoing_copy = outgoing.try_clone().unwrap();
                                let copy = thread::spawn(move || {
                                    let _ = io::copy(&mut incoming_copy, &mut outgoing_copy);
                                    let _ = outgoing_copy.shutdown(Shutdown::Write);
                                });
                                let _ = io::copy(&mut outgoing, &mut incoming);
                                let _ = incoming.shutdown(Shutdown::Write);
                                let _ = copy.join();
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(_) => break,
                    }
                }
            });
            *forward = Some((instance.id.clone(), address));
            self.forwards.fetch_add(1, Ordering::AcqRel);
        }
        self.prepare_done.store(true, Ordering::Release);
        drop(forward);
        self.note("prepare-exit");
        Ok(())
    }
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.note("dial-through-forward");
        let forward = self.forward.lock().unwrap();
        let (id, address) = forward.as_ref().expect("dial must have a prepared forward");
        assert_eq!(id, &instance.id);
        TcpStream::connect_timeout(address, Duration::from_secs(2))
            .map_err(|_| Failure::Unreachable)
    }
}
fn start(route: Arc<Probe>, lab: &super::Lab) -> Observer {
    let observer = Observer::start_on(
        route.clone(),
        &lab.data,
        lab.requests.clone(),
        Arc::new(|| {}),
        quick(),
    )
    .unwrap();
    route.install(&observer.local());
    observer
}

#[test]
fn reverify25_prepare_builds_a_reusable_forward_and_every_route_call_is_outside_model_lock() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Probe::at(&lab.home, None, false));
    let observer = start(route.clone(), &lab);
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    lab.requests
        .answer(
            &id,
            &json!({"kind":"answer", "answers":{"715a2db63d":{"selected":["green"]}}}),
            crate::hooks::now_ms(),
        )
        .unwrap();
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    until("all ticketed sockets reached the fake gateway", || {
        let world = gateway.world();
        world.sockets == world.granted.len()
    });
    drop(observer);
    let events = route.events.lock().unwrap().clone();
    eprintln!(
        "FORWARD count={} ticketed_sockets={} events={}",
        route.forwards.load(Ordering::Acquire),
        gateway.world().sockets,
        json!(events)
    );
    assert_eq!(route.forwards.load(Ordering::Acquire), 1);
    assert!(
        events
            .iter()
            .filter(|e| e.starts_with("dial-through-forward"))
            .count()
            >= 2
    );
}

#[test]
fn reverify25_failed_prepare_becomes_unreachable_without_ticket_or_dial() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Probe::at(&lab.home, None, true));
    let observer = start(route.clone(), &lab);
    until("prepare failure reported", || {
        lab.log().contains("connection failed reason=Unreachable")
    });
    drop(observer);
    assert!(gateway.world().granted.is_empty());
    assert_eq!(gateway.world().sockets, 0);
    eprintln!(
        "PREPARE FAILURE\n{}EVENTS {}",
        lab.log(),
        json!(route.events.lock().unwrap().clone())
    );
}

#[test]
fn reverify25_stop_cancels_an_inflight_prepare_and_prevents_later_ticketing() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let gate = Arc::new(Hold::default());
    let route = Arc::new(Probe::at(&lab.home, Some(gate.clone()), false));
    let observer = start(route.clone(), &lab);
    let shared = observer.local();
    until("prepare entered", || gate.entered.load(Ordering::Acquire));
    assert!(shared.model.try_lock().is_ok());
    let began = Instant::now();
    drop(observer);
    let stop_ms = began.elapsed().as_millis();
    thread::sleep(Duration::from_millis(250));
    let cancelled = route.prepare_done.load(Ordering::Acquire);
    let tickets_at_stop = gateway.world().granted.len();
    let still_active = shared.active();
    // Always release the test's gate and wait for the worker, even when the assertion fails.
    gate.released.store(true, Ordering::Release);
    until("prepare completed after test-only release", || {
        route.prepare_done.load(Ordering::Acquire)
    });
    until("stopped worker returned", || {
        Arc::strong_count(&shared) == 1
    });
    let tickets_after_release = gateway.world().granted.len();
    let events = route.events.lock().unwrap().clone();
    eprintln!("STOP stop_ms={stop_ms} active={still_active} prepare_cancelled_within_250ms={cancelled} tickets_at_stop={tickets_at_stop} tickets_after_release={tickets_after_release}\nEVENTS {}", json!(events));
    assert!(
        cancelled,
        "stop never cancels the in-flight prepare; only the verifier's manual release ends it"
    );
    assert_eq!(
        tickets_after_release, tickets_at_stop,
        "a stopped observer requested a fresh ticket after prepare returned"
    );
    assert_eq!(
        route.forwards.load(Ordering::Acquire),
        0,
        "a forward was made for a stopped observer"
    );
    let began = events
        .iter()
        .position(|e| e.starts_with("prepare-enter"))
        .unwrap();
    let after: Vec<&String> = events[began + 1..].iter().collect();
    assert_eq!(after.len(), 1, "{after:?}");
    assert!(
        after[0].starts_with("prepare-cancelled active=false"),
        "{after:?}"
    );
}
