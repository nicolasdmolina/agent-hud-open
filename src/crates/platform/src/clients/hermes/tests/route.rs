//! The route the tests follow, and what the connection asks of a route:
//! everything. It has no way to a gateway of its own, and an origin that
//! is not this machine is refused before anything is dialled.
use super::super::{
    control::Silent,
    route::{local::Local, Home, Route, Stop},
    socket::Socket,
};
use super::{
    fake::{Fake, Hold},
    lab, until,
};
use agent_hud_agents::hermes::gateway::{
    granted, identify, identify_line, ticket_line, Failure, Identified, Instance, Scope,
};
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    io::Read,
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// The way to the pretend gateway: the home it was started in, reached as
/// the local route reaches a real one, with everything asked of the route
/// kept, and a way to shut it.
pub struct Way {
    local: Local,
    /// A route that is shut tells of no home, as one whose machine cannot
    /// be reached.
    pub shut: AtomicBool,
    /// Holds `prepare`, or a stream that `dial` has just got, until
    /// released, whether or not the one who asked has stopped: a route
    /// that does not listen.
    pub held: Mutex<Option<(&'static str, Arc<Hold>)>>,
    /// Where `dial` leads in place of the gateway: somewhere that takes
    /// the stream and never says a word on it.
    pub mute: Mutex<Option<SocketAddr>>,
    pub asked: Mutex<Asked>,
}

/// What was asked of a route, in order.
#[derive(Default)]
pub struct Asked {
    /// How many times the homes were asked for.
    pub homes: usize,
    /// The verb of each exchange with a control socket.
    pub verbs: Vec<String>,
    /// The origin of each instance the route was told to prepare for.
    pub prepared: Vec<String>,
    /// The origin of each instance a stream was asked to.
    pub dials: Vec<String>,
}

impl Way {
    pub fn to(home: &Path) -> Self {
        Self {
            local: Local::at(home),
            shut: AtomicBool::new(false),
            held: Mutex::default(),
            mute: Mutex::default(),
            asked: Mutex::default(),
        }
    }

    pub fn asked(&self) -> std::sync::MutexGuard<'_, Asked> {
        self.asked.lock().unwrap()
    }

    /// Waits at `step` while it is held.
    fn hold(&self, step: &str) {
        let held = self.held.lock().unwrap().clone();
        if let Some((_, held)) = held.filter(|(at, _)| *at == step) {
            held.entered.store(true, Ordering::Release);
            while !held.released.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
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

    fn named(&self, home: &Home) -> Cow<'_, str> {
        self.local.named(home)
    }

    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        self.asked().prepared.push(instance.origin.clone());
        self.hold("prepare");
        self.local.prepare(instance, stop)
    }

    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.asked().dials.push(instance.origin.clone());
        let mute = *self.mute.lock().unwrap();
        let stream = match mute {
            Some(address) => TcpStream::connect(address).map_err(|_| Failure::Unreachable),
            None => self.local.dial(instance),
        };
        self.hold("dial");
        stream
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

    // What the route was asked for, beside what the gateway gave: the
    // tickets asked for and granted, the streams asked for and the sockets
    // opened. The observer still runs, and a poll may be taking a snapshot
    // on a socket of its own just now, so the two are looked at together
    // until they agree. (They need not once it has stopped: an opening
    // that a stop overtakes asks for nothing more.)
    struct Seen {
        verbs: Vec<String>,
        prepared: Vec<String>,
        dials: Vec<String>,
        homes: usize,
        granted: usize,
        sockets: usize,
        turned_away: usize,
    }
    let seen = || {
        let (world, asked) = (gateway.world(), lab.route.asked());
        Seen {
            verbs: asked.verbs.clone(),
            prepared: asked.prepared.clone(),
            dials: asked.dials.clone(),
            homes: asked.homes,
            granted: world.granted.len(),
            sockets: world.sockets,
            turned_away: world.turned_away,
        }
    };
    let tickets = |seen: &Seen| {
        let ticket = |verb: &&String| *verb == "session-ticket";
        seen.verbs.iter().filter(ticket).count()
    };
    let mut last = seen();
    until("the gateway gave what the route was asked for", || {
        last = seen();
        let asked_for = (tickets(&last), last.dials.len());
        asked_for == (last.granted, last.sockets + last.turned_away)
            && asked_for.0 == asked_for.1
            && last.prepared.len() == asked_for.0
    });
    lab.observer.take();
    // The socket the polls run on and the one the answer went on, at least:
    // a ticket and a stream each, and every one of them asked of the route.
    assert!(tickets(&last) >= 2, "{}", tickets(&last));
    // The route was told of the instance before each ticket was asked for.
    assert_eq!(last.prepared, last.dials);
    assert_eq!(last.turned_away, 0);
    assert_eq!(last.verbs[0], "identify");
    let origin = gateway.world().identify.as_ref().unwrap()["result"]["api_origin"].clone();
    assert!(last.dials.iter().all(|dialled| json!(dialled) == origin));
    // A home is asked for before each thing asked in it.
    assert!(last.homes >= last.verbs.len(), "{}", last.homes);
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
fn once_the_observer_has_stopped_no_ticket_is_asked_for_whatever_the_route_answers() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("prepare", held.clone()));
    lab.follow();
    until("the route is told of the instance", || {
        held.entered.load(Ordering::Acquire)
    });
    // Stopped while the route still prepares. This route does not listen
    // for that, and says all is well once it is let go.
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    std::thread::sleep(Duration::from_millis(300));
    let (verbs, dials) = {
        let asked = lab.route.asked();
        (asked.verbs.clone(), asked.dials.clone())
    };
    assert_eq!(verbs, ["identify"]);
    assert!(dials.is_empty(), "{dials:?}");
    let (granted, sockets) = {
        let world = gateway.world();
        (world.granted.len(), world.sockets + world.turned_away)
    };
    assert_eq!((granted, sockets), (0, 0));
}

#[test]
fn a_stream_that_arrives_once_the_observer_stopped_is_never_spoken_on() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("dial", held.clone()));
    lab.follow();
    until("a stream is dialled", || {
        held.entered.load(Ordering::Acquire)
    });
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    // Stopped with the ticket in hand and the stream on its way.
    lab.observer.take();
    held.released.store(true, Ordering::Release);
    until("the connection's thread is done", || {
        Arc::strong_count(&shared) == 1
    });
    std::thread::sleep(Duration::from_millis(100));
    // The ticket was never offered: the gateway saw no upgrade at all.
    let (granted, upgrades) = {
        let world = gateway.world();
        (world.granted.len(), world.sockets + world.turned_away)
    };
    assert_eq!((granted, upgrades), (1, 0));
    let dials = lab.route.asked().dials.len();
    assert_eq!(dials, 1);
}

#[test]
fn stopping_cuts_a_stream_that_is_being_upgraded() {
    let mut lab = lab();
    let _gateway = Fake::start(&lab.home, "instance-1");
    // The stream leads somewhere that takes it and never answers, so the
    // upgrade waits: five seconds, were it left alone.
    let mute = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    *lab.route.mute.lock().unwrap() = Some(mute.local_addr().unwrap());
    lab.follow();
    let (mut taken, _) = mute.accept().unwrap();
    let mut asked = [0u8; 4096];
    let read = taken.read(&mut asked).unwrap();
    assert!(
        asked[..read].starts_with(b"GET "),
        "the upgrade is under way"
    );
    let shared = lab.observer.as_ref().unwrap().shared.clone();
    let began = Instant::now();
    lab.observer.take();
    until("the connection's thread is done", || {
        Arc::strong_count(&shared) == 1
    });
    let took = began.elapsed();
    assert!(took < Duration::from_secs(2), "{took:?}");
    // The other end sees the stream end, with nothing more on it.
    taken
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(taken.read(&mut asked).ok(), Some(0));
}

#[test]
fn the_local_route_names_its_home_with_links_resolved_and_none_where_there_is_none() {
    let lab = lab();
    let link = lab.data.join("link");
    std::os::unix::fs::symlink(&lab.home, &link).unwrap();
    let route = Local::at(&link);
    let homes = route.homes().unwrap();
    assert_eq!(homes.len(), 1);
    assert_eq!(route.named(&homes[0]), lab.home.to_string_lossy());
    let missing = Local::at(lab.data.join("missing")).homes();
    assert_eq!(missing, Err(Silent::Absent));
    // The name is where the home leads now, not where it led when the
    // homes were asked for.
    let other = lab.data.join("other");
    crate::os::private_directory(&other).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    assert_eq!(route.named(&homes[0]), other.to_string_lossy());
    // A home that leads nowhere any more goes by the name it was given.
    std::fs::remove_file(&link).unwrap();
    assert_eq!(route.named(&homes[0]), link.to_string_lossy());
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
