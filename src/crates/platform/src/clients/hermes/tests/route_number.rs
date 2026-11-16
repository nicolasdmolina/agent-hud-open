//! The route number a follower's polls are read under (ticket 34): the
//! one its socket was opened under, from before the home was asked who
//! is there until the socket was open. A socket opened while the number
//! changed is of no one route: it is closed with no poll run on it, and
//! opened again.
use super::{
    super::{
        control::Silent,
        route::{Call, Home, Inventory, Route, Stop},
        Observer,
    },
    fake::Fake,
    lab, quick,
    route::Way,
    until,
};
use agent_hud_agents::hermes::gateway::{Failure, Instance};
use std::{
    borrow::Cow,
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

/// The tests' route with a number, which goes up once, in the last step
/// of opening a socket: after every step that could have noticed.
struct Numbered {
    way: Arc<Way>,
    number: AtomicU64,
    raise_at_admit: AtomicBool,
}

impl Route for Numbered {
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
        self.way.dial(instance)
    }
    fn number(&self) -> u64 {
        self.number.load(Ordering::Acquire)
    }
    fn admit(&self, _: &Instance, _: Call<'_>) -> Result<(), Failure> {
        if self.raise_at_admit.swap(false, Ordering::AcqRel) {
            self.number.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

#[test]
fn a_socket_opened_while_the_route_number_changed_is_closed_and_opened_again() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Numbered {
        way: lab.route.clone(),
        number: AtomicU64::new(1),
        raise_at_admit: AtomicBool::new(true),
    });
    let wake = Arc::new(|| {});
    let requests = lab.requests.clone();
    let observer = Observer::start_on(route.clone(), &lab.data, requests, wake, quick()).unwrap();
    until("the polls run", || !gateway.world().listed.is_empty());

    // The first socket was opened under 1 and admitted under 2: nothing
    // was asked on it. The polls run on the second, opened under 2.
    assert_eq!(route.number(), 2);
    let follower = observer.local();
    assert_eq!(follower.under.load(Ordering::Acquire), 2);
    let world = gateway.world();
    assert!(world.sockets >= 2, "{} sockets", world.sockets);
    let on_the_first = |(socket, _): &&(usize, serde_json::Value)| *socket == 1;
    let asked = world.received.iter().filter(on_the_first).count();
    assert_eq!(asked, 0, "{:?}", world.received);
    assert!(!world.calls("session.active_list").is_empty());
}

/// A route that is one way for its whole life has the number 0, and the
/// polls of its followers are read under it.
#[test]
fn a_route_that_never_changes_has_the_number_nought() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    until("the polls run", || !gateway.world().listed.is_empty());
    assert_eq!(lab.route.number(), 0);
    let follower = lab.observer.as_ref().unwrap().local();
    assert_eq!(follower.under.load(Ordering::Acquire), 0);
}
