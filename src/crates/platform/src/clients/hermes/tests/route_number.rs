//! The route number a follower's polls are read under (ticket 34): the
//! one its socket was opened under, from before the home was asked who
//! is there until the socket was open. A socket opened while the number
//! changed is of no one route: it is closed with no poll run on it, and
//! opened again.
use super::{
    super::{
        control::Silent,
        followers::Followers,
        log::Log,
        mirror::{Routes, Writer},
        route::{Call, Home, Inventory, Route, Stop},
        Observer,
    },
    fake::Fake,
    lab,
    machines::mirrors::works,
    quick,
    route::Way,
    until,
};
use crate::hooks::Changed;
use agent_hud_agents::hermes::gateway::{machine_key, Failure, Instance, Machine, Profile};
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

/// The machine's route number, asked as its mirrors' writer asks the
/// switch of a real machine.
struct Asked(Arc<Numbered>);

impl Routes for Asked {
    fn under(&self, number: u64, commit: &mut dyn FnMut()) -> bool {
        let current = self.0.number() == number;
        if current {
            commit();
        }
        current
    }
}

/// What a poll listed is queued for the mirror under the number its
/// socket was opened under, not under the route's number at that
/// moment (ticket 54; ticket 34's verifier, finding 5). Here the number
/// goes up and nothing else happens: the follower is told of no change,
/// so its model takes the next poll like any other, and the number the
/// listing carries is all that keeps it out of the mirror.
#[test]
fn a_poll_is_written_under_the_route_number_its_socket_was_opened_under() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Numbered {
        way: lab.route.clone(),
        number: AtomicU64::new(1),
        raise_at_admit: AtomicBool::new(false),
    });
    let key = machine_key(Some("0123456789abcdef0123456789abcdef"), "/far/.hermes").unwrap();
    let untold: Changed = Arc::new(|_| {});
    let log = Log::new(&lab.data).of(Some("mirror"));
    let asked = Arc::new(Asked(route.clone()));
    let writer = Writer::start(&lab.data, (&key, "Mini"), asked, untold, log).unwrap();
    let far = (Machine::Far(key), Some("Mini".to_owned()));
    let (requests, wake) = (lab.requests.clone(), Arc::new(|| {}));
    let mirror = Some(writer.clone());
    let followers = Followers::start(
        route.clone(),
        far,
        &lab.data,
        requests,
        wake,
        quick(),
        (mirror, None),
    )
    .unwrap();
    let rows = || {
        let path = writer.file(&Profile::Default);
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let Ok(db) = rusqlite::Connection::open_with_flags(path, flags) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = (db.prepare("SELECT id FROM sessions ORDER BY id"))
            .and_then(|mut all| all.query_map([], |row| row.get(0))?.collect())
            .unwrap_or_default();
        ids.sort();
        ids
    };

    // A session in a turn: the polls, on a socket opened under 1, list
    // it, and it gets its row.
    works(&gateway, "s-before", "Before the route changed");
    until("its row is written", || rows() == ["s-before"]);

    // The number goes up. The socket is the one opened under 1 still.
    route.number.store(2, Ordering::Release);
    works(&gateway, "s-after", "After the route changed");
    // The poll that begins next lists it; once another has begun, that
    // one has ended and queued what it listed. A poll lists each of the
    // gateway's two profiles, so two polls are four lists: with two, the
    // poll that lists it may only have begun.
    let seen = gateway.world().listed.len();
    until("two more polls begin", || {
        gateway.world().listed.len() >= seen + 4
    });
    writer.settle();
    let under: Vec<u64> = (followers.set().iter())
        .map(|follower| follower.under.load(Ordering::Acquire))
        .collect();
    assert_eq!(under, [1]);
    assert_eq!(rows(), ["s-before"], "written under the number as it is");
    let log = std::fs::read_to_string(lab.data.join("hermes-gateway.log")).unwrap();
    assert!(
        log.contains("mirror dropped profile=default why=route"),
        "{log}"
    );
    followers.stop();
}
