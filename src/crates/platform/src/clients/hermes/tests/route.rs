//! The route the tests follow, and what the connection asks of a route:
//! everything. It has no way to a gateway of its own, and an origin that
//! is not this machine is refused before anything is dialled.
use super::super::{
    control::Silent,
    route::{local::Local, Home, Route},
    socket::Socket,
};
use super::{fake::Fake, lab, until};
use agent_hud_agents::hermes::gateway::{
    granted, identify, identify_line, ticket_line, Failure, Identified, Instance, Scope,
};
use serde_json::{json, Value};
use std::{
    net::{IpAddr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

/// The way to the pretend gateway: the home it was started in, reached as
/// the local route reaches a real one, with everything asked of the route
/// kept, and a way to shut it.
pub struct Way {
    local: Local,
    /// A route that is shut tells of no home, as one whose machine cannot
    /// be reached.
    pub shut: AtomicBool,
    pub asked: Mutex<Asked>,
}

/// What was asked of a route, in order.
#[derive(Default)]
pub struct Asked {
    /// How many times the homes were asked for.
    pub homes: usize,
    /// The verb of each exchange with a control socket.
    pub verbs: Vec<String>,
    /// The origin of each instance a stream was asked to.
    pub dials: Vec<String>,
}

impl Way {
    pub fn to(home: &Path) -> Self {
        Self {
            local: Local::at(home),
            shut: AtomicBool::new(false),
            asked: Mutex::default(),
        }
    }

    pub fn asked(&self) -> std::sync::MutexGuard<'_, Asked> {
        self.asked.lock().unwrap()
    }
}

impl Route for Way {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.asked().homes += 1;
        if self.shut.load(Ordering::Acquire) {
            return Err(Silent::Absent);
        }
        self.local.homes()
    }

    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        let said: Value = serde_json::from_slice(line).unwrap_or_default();
        let verb = said["verb"].as_str().unwrap_or("").to_owned();
        self.asked().verbs.push(verb);
        self.local.exchange(home, line, timeout)
    }

    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.asked().dials.push(instance.origin.clone());
        self.local.dial(instance)
    }
}

#[test]
fn everything_the_connection_asks_of_a_gateway_goes_through_its_route() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    lab.observer.take();

    // What the route was asked for, beside what the gateway gave: the
    // tickets asked for and granted, the streams asked for and the sockets
    // opened. A poll may have taken a snapshot besides, and a visit still
    // on its way when the observer stopped ends on its own, so the two are
    // looked at until they agree.
    let seen = || {
        let (world, asked) = (gateway.world(), lab.route.asked());
        let tickets = |verb: &&String| *verb == "session-ticket";
        let asked_for = (
            asked.verbs.iter().filter(tickets).count(),
            asked.dials.len(),
        );
        let given = (world.granted.len(), world.sockets + world.turned_away);
        (asked_for, given)
    };
    until("the gateway gave what the route was asked for", || {
        let (asked_for, given) = seen();
        asked_for == given && asked_for.0 == asked_for.1
    });
    // The socket the polls run on and the one the answer went on, at least:
    // a ticket and a stream each, and every one of them asked of the route.
    let ((tickets, dials), _) = seen();
    assert!(tickets >= 2 && dials >= 2, "{tickets} {dials}");
    let origin = gateway.world().identify.as_ref().unwrap()["result"]["api_origin"].clone();
    let (verbs, dialled, homes) = {
        let asked = lab.route.asked();
        (asked.verbs.clone(), asked.dials.clone(), asked.homes)
    };
    assert_eq!(gateway.world().turned_away, 0);
    assert_eq!(verbs[0], "identify");
    assert!(dialled.iter().all(|dialled| json!(dialled) == origin));
    // A home is asked for before each thing asked in it.
    assert!(homes >= verbs.len(), "{homes} {verbs:?}");
}

#[test]
fn a_route_that_is_shut_reaches_no_gateway_though_one_runs() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.route.shut.store(true, Ordering::Release);
    lab.follow();
    until("no gateway is found", || {
        lab.log().contains("connection failed reason=Unreachable")
    });
    std::thread::sleep(Duration::from_millis(200));
    let (granted, dialled) = {
        let world = gateway.world();
        (world.granted.len(), world.sockets + world.turned_away)
    };
    let (verbs, dials, homes) = {
        let asked = lab.route.asked();
        (asked.verbs.clone(), asked.dials.clone(), asked.homes)
    };
    assert_eq!((granted, dialled), (0, 0));
    assert!(verbs.is_empty(), "{verbs:?}");
    assert!(dials.is_empty(), "{dials:?}");
    assert!(homes > 0);
    assert!(!lab.log().contains("connection ready"), "{}", lab.log());
    // The same observer finds it once the route leads there.
    lab.route.shut.store(false, Ordering::Release);
    until("the gateway is found", || gateway.world().sockets == 1);
}

#[test]
fn the_local_route_names_its_home_with_links_resolved_and_none_where_there_is_none() {
    let lab = lab();
    let link = lab.data.join("link");
    std::os::unix::fs::symlink(&lab.home, &link).unwrap();
    let homes = Local::at(&link).homes().unwrap();
    assert_eq!(homes.len(), 1);
    assert_eq!(homes[0].name(), lab.home.to_string_lossy());
    assert_eq!(homes, Local::at(&lab.home).homes().unwrap());
    let missing = Local::at(lab.data.join("missing")).homes();
    assert_eq!(missing, Err(Silent::Absent));
}

/// An address of this machine that is not loopback, where it has one.
fn outward() -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // Nothing is sent: connecting only picks the address to send from.
    socket.connect(("192.0.2.1", 9)).ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_loopback() && !address.is_unspecified()).then_some(address)
}

#[test]
fn an_origin_that_is_not_this_machine_is_refused_before_any_dial() {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let wait = Duration::from_secs(5);
    let told = super::ask(&lab.home, &identify_line(), wait).unwrap();
    let Identified::Ready(ready) = identify(&told, lab.home.to_str().unwrap()) else {
        panic!("not ready")
    };
    // Something that would take the dial, were one made: a listener on
    // every address of this machine.
    let listener = TcpListener::bind(("0.0.0.0", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut origins = vec![format!("http://192.0.2.1:{port}")];
    origins.extend(outward().map(|address| match address {
        IpAddr::V4(address) => format!("http://{address}:{port}"),
        IpAddr::V6(address) => format!("http://[{address}]:{port}"),
    }));
    eprintln!("origins that are not this machine: {origins:?}");
    let refused = Some(Failure::Invalid("invalid_api_origin"));
    for origin in origins {
        let far = Instance {
            origin: origin.clone(),
            port,
            ..ready.clone()
        };
        let route = Way::to(&lab.home);
        assert_eq!(route.dial(&far).err(), refused, "{origin}");
        // The same through the socket, with a ticket in hand.
        let grant = super::ask(&lab.home, &ticket_line(&ready, Scope::Host), wait).unwrap();
        let ticket = granted(&grant, &ready).unwrap();
        let opened = Socket::open(&route, &far, ticket);
        assert_eq!(opened.err(), refused, "{origin}");
        let dials = route.asked().dials.clone();
        assert_eq!(dials, [origin.clone(), origin.clone()]);
        assert!(listener.accept().is_err(), "{origin} was dialled");
    }
    let dialled = {
        let world = gateway.world();
        world.sockets + world.turned_away
    };
    assert_eq!(dialled, 0);
    // The gateway's own origin, on this machine, is dialled.
    let route = Way::to(&lab.home);
    let grant = super::ask(&lab.home, &ticket_line(&ready, Scope::Host), wait).unwrap();
    let mut socket = Socket::open(&route, &ready, granted(&grant, &ready).unwrap()).unwrap();
    socket.close();
    assert_eq!(gateway.world().sockets, 1);
}
