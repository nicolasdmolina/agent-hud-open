//! The link a reading was made over (ticket 58), without an `sshd`: the
//! tests' route with a link of its own that a test loses or replaces.
//! What a poll, or a reading of one session apart, read over a link that
//! is gone by the time the model would take it is dropped, so it never
//! takes a mark off a card; and a stream dialled while the link changed
//! is of no one link. The same ordering over a real link, as ticket 56
//! found it, is in `far/late.rs`.
use super::{
    super::{
        control::Silent,
        route::{Call, Epoch, Home, Inventory, Route, Stop},
        socket::Socket,
        Observer, Shared,
    },
    fake::{Fake, PROFILE},
    lab, marked, quick,
    route::Way,
    until, Lab, Pace,
};
use agent_hud_agents::hermes::gateway::{Connection, Failure, Input, Instance};
use serde_json::json;
use std::{
    borrow::Cow,
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

/// The tests' route over a link with a number: 0 is no link at all.
struct Linked {
    way: Arc<Way>,
    link: AtomicU64,
    /// The link gives way to the next while a stream is dialled, once.
    replaced_at_dial: AtomicBool,
}

impl Linked {
    fn over(lab: &Lab) -> Arc<Self> {
        Arc::new(Self {
            way: lab.route.clone(),
            link: AtomicU64::new(1),
            replaced_at_dial: AtomicBool::new(false),
        })
    }

    fn set(&self, link: u64) {
        self.link.store(link, Ordering::Release);
    }
}

impl Route for Linked {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.way.homes()
    }
    fn inventory(&self) -> Result<Inventory, Silent> {
        self.way.inventory()
    }
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        self.way.exchange(home, line, timeout)
    }
    fn named(&self, home: &Home) -> Cow<'_, str> {
        self.way.named(home)
    }
    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        self.way.prepare(instance, stop)
    }
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        let dialled = self.way.dial(instance);
        if self.replaced_at_dial.swap(false, Ordering::AcqRel) {
            self.link.fetch_add(1, Ordering::AcqRel);
        }
        dialled
    }
    fn epoch(&self) -> Option<Epoch> {
        match self.link.load(Ordering::Acquire) {
            0 => None,
            link => Some(Epoch::of(link)),
        }
    }
    fn admit(&self, _: &Instance, _: Call<'_>) -> Result<(), Failure> {
        Ok(())
    }
}

fn follow(lab: &Lab, route: &Arc<Linked>, pace: Pace) -> (Observer, Arc<Shared>) {
    let (requests, wake) = (lab.requests.clone(), Arc::new(|| {}));
    let observer = Observer::start_on(route.clone(), &lab.data, requests, wake, pace).unwrap();
    until("a follower", || !observer.followers().is_empty());
    let follower = observer.local();
    (observer, follower)
}

/// An answer to the one card could not be sent: the card is marked.
fn unsent(lab: &Lab, follower: &Shared) {
    let card = lab.card();
    let request = card["id"].as_str().unwrap();
    follower.apply(Input::Unsent { request });
    until("the card is marked", || lab.cards().iter().all(marked));
}

fn lost(lab: &Lab) -> usize {
    lab.log().matches("connection failed reason=").count()
}

fn reached(lab: &Lab) -> usize {
    lab.log().matches(" reached id=").count()
}

/// A poll read over link 1 comes back once the card is marked and the
/// link is `now` (0: lost): it is dropped with its socket, and the mark
/// stays until a poll read over a link that stands finds the request.
fn a_late_poll_is_dropped(now: u64) {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Linked::over(&lab);
    let (_observer, follower) = follow(&lab, &route, quick());
    gateway.play("question-answered-here", Some("clarify.request"));
    lab.card();
    let listed = gateway.world().listed.len();
    until("a poll began", || gateway.world().listed.len() > listed);
    follower.late.arm();
    until("a poll has read its replies", || follower.late.held());
    unsent(&lab, &follower);
    route.set(now);
    let before = lost(&lab);
    follower.late.free();
    // Dropped, and its socket given up as lost; taken, the log says so.
    until("the poll came back", || {
        lost(&lab) > before || reached(&lab) > 0
    });
    let log = lab.log();
    let until_lost = log.rsplit_once("connection failed reason=");
    let until_lost = until_lost.map_or(log.as_str(), |(before, _)| before);
    assert!(
        lost(&lab) > before && !until_lost.contains(" reached id="),
        "a poll read over link 1 was taken with the link at {now}:\n{log}"
    );
    if now == 0 {
        // No link, no reading: the mark stays for as long as that lasts.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(reached(&lab), 0, "{}", lab.log());
        assert!(lab.cards().iter().all(marked));
        route.set(2);
    }
    until("a poll over the link that stands reaches the card", || {
        lab.cards().len() == 1 && !lab.cards().iter().any(marked)
    });
    assert_eq!(reached(&lab), 1, "{}", lab.log());
}

#[test]
fn a_poll_read_over_a_link_since_lost_never_clears_a_mark() {
    a_late_poll_is_dropped(0);
}

#[test]
fn a_poll_read_over_a_link_since_replaced_never_clears_a_mark() {
    a_late_poll_is_dropped(2);
}

const SLOW: &str = "s-m-slow";

#[test]
fn a_reading_apart_made_over_a_link_since_lost_never_clears_a_mark() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    gateway.tells_quietly(PROFILE, SLOW, "message.start", json!({}));
    gateway.world().slow_replay = Some((SLOW.into(), Duration::from_millis(400)));
    let route = Linked::over(&lab);
    let apart = Duration::from_millis(150);
    let (_observer, follower) = follow(&lab, &route, Pace { apart, ..quick() });
    until("the slow session is read apart", || {
        follower.apart.count() == 1
    });
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    lab.card();
    // A reading begun now reads the session whole and finds the request.
    let read = || gateway.world().calls("session.events.since").len();
    let before = read();
    until("a reading began", || read() > before);
    follower.late.arm_apart();
    until("a reading has read its replies", || follower.late.held());
    unsent(&lab, &follower);
    route.set(0);
    follower.late.free();
    // The lane ends, with no link to open a socket over.
    until("the reading came back", || {
        lab.log().contains("together session=") || reached(&lab) > 0
    });
    assert_eq!(reached(&lab), 0, "taken with no link:\n{}", lab.log());
    assert!(lab.cards().iter().all(marked));
    route.set(2);
    until(
        "a reading over the link that stands reaches the card",
        || lab.cards().len() == 1 && !lab.cards().iter().any(marked),
    );
    assert_eq!(reached(&lab), 1, "{}", lab.log());
}

#[test]
fn a_stream_dialled_while_the_link_changed_is_of_no_link() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Linked::over(&lab);
    let (_observer, follower) = follow(&lab, &route, quick());
    until("the polls run", || !gateway.world().listed.is_empty());
    let Connection::Ready(instance) = follower.model().gateway.connection().clone() else {
        panic!("not ready");
    };
    // The link stands before and after: a stream.
    assert!(Socket::dial(&*route, &instance).is_ok());
    // It gave way to the next while the stream was dialled.
    route.replaced_at_dial.store(true, Ordering::Release);
    let dialled = Socket::dial(&*route, &instance);
    assert_eq!(dialled.err(), Some(Failure::Unreachable));
    assert_eq!(route.epoch(), Some(Epoch::of(2)));
    // No link: nothing is dialled at all.
    route.set(0);
    let dials = route.way.asked().dials.len();
    let dialled = Socket::dial(&*route, &instance);
    assert_eq!(dialled.err(), Some(Failure::Unreachable));
    assert_eq!(route.way.asked().dials.len(), dials);
}
