//! The fourth review of ticket 26 (`evidence/26/verifier-report-4.md`):
//! its probes (`verifier-files-4/round4.rs`), tracked as tests, as the
//! verifier wrote them. Two failed on `128a5a9`: a `profiles` folder
//! that cannot be read was taken for word that a home is gone, and a
//! folder with no gateway that cannot be entered kept every contest for
//! ever. The others passed there and pass now: the name fallback, the
//! gate of a follower back from gone, and two claimants that are unclear
//! and keep their claims.
//!
//! One probe is left out: a live listener with a full queue, which macOS
//! answers as a refused connection. That is an accepted residual, written
//! in `FORK.md`, and its probe fails by design.
use super::super::{
    control::{self, Silent},
    route::{local::Local, Home, Route, Stop},
    Observer, Pace,
};
use super::{
    fake::{home_of, Fake},
    lab, quick,
    route::Way,
    until, Lab,
};
use agent_hud_agents::hermes::gateway::{identify_line, Failure, Instance};
use std::{
    borrow::Cow,
    net::TcpStream,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn contest(lab: &mut Lab) -> (Fake, Fake) {
    let first = Fake::serving(&lab.home, "aaaaaaaa-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    second.serve(&["beta", "alpha"]);
    first.asks("default", "s-default", "p2");
    first.asks("alpha", "s-alpha", "p1");
    second.asks("beta", "s-beta", "p3");
    second.asks("alpha", "s-other", "p4");
    lab.follow_at(Pace {
        discover: Some(Duration::from_millis(40)),
        ..quick()
    });
    until("contest and uncontested cards", || {
        lab.cards().len() == 2 && lab.said("contested") == ["contested profiles=[alpha]"]
    });
    lab.looks(4);
    (first, second)
}

fn looks_for(lab: &Lab, time: Duration) {
    let start = Instant::now();
    while start.elapsed() < time {
        lab.looks(5);
    }
}

/// Finding A. The `profiles` folder cannot be read (EACCES). The route
/// still tells of the homes "successfully", with only the root in the
/// list, and the follower of a live gateway that lost its socket takes
/// that for word that its home is gone.
#[test]
fn round4_an_unreadable_profiles_folder_is_not_word_that_a_home_is_gone() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    let profiles = lab.home.join("profiles");
    let beta = home_of(&lab.home, "beta");
    std::fs::set_permissions(&profiles, std::fs::Permissions::from_mode(0o000)).unwrap();
    let raw = std::fs::read_dir(&profiles).map(|_| ()).unwrap_err();
    let told: Vec<String> = (lab.route.homes().unwrap().iter())
        .map(|home| home.profile().name().to_owned())
        .collect();
    second.cut();
    looks_for(&lab, Duration::from_secs(2));
    let gone = follower.gone.load(Ordering::Acquire);
    let cards = lab.shown();
    eprintln!(
        "UNREADABLE_PROFILES raw={raw:?} homes_told={told:?} gone={gone} active={} cards={cards:?} contests={:?}\n{}",
        follower.active(),
        lab.said("contested"),
        lab.log()
    );
    // The same listener answers once the folder can be read again: it
    // never stopped.
    std::fs::set_permissions(&profiles, std::fs::Permissions::from_mode(0o700)).unwrap();
    let route = Local::at(&beta);
    let home = route.homes().unwrap().remove(0);
    let answered = control::ask(&route, &home, &identify_line(), Duration::from_secs(2));
    eprintln!(
        "UNREADABLE_PROFILES same_listener_answers={}",
        answered.is_ok()
    );
    assert!(answered.is_ok(), "the listener never stopped");
    assert!(
        !gone,
        "a profiles folder that cannot be read says nothing of which homes there are"
    );
    assert!(
        !cards.iter().any(|id| id.contains("alpha:s-alpha:p1")),
        "a permission error on the inventory must not release a live claimant's profile"
    );
}

/// The name fallback: two newcomers told apart alike, found in one look,
/// get the same names whichever home is asked first; the names and the
/// follower objects stay through later looks; and one of them ending
/// leaves the other's name, object and card as they were.
fn collide(backwards: bool) -> (Vec<String>, Vec<String>) {
    let mut lab = lab();
    lab.route.backwards.store(backwards, Ordering::Release);
    let _default = Fake::serving(&lab.home, "root", &["default"]);
    let original = Fake::serving(&lab.home, "original", &["alpha", "zeta"]);
    original.asks("zeta", "s-old", "p0");
    lab.follow_all();
    until("original followed", || lab.cards().len() == 1);
    original.serve(&["zeta"]);
    until("original relinquishes alpha", || {
        !home_of(&lab.home, "alpha").join("gateway.sock").exists()
    });
    lab.looks(8);
    let one = "newcomer-000000000000000000000000000112df";
    let two = "newcomer-00000000000000000000000000028885";
    // No look while both start, so one look finds both.
    lab.route.shut.store(true, Ordering::Release);
    lab.looks(2);
    let first = Fake::serving(&lab.home, one, &["alpha", "beta"]);
    let second = Fake::serving(&lab.home, two, &["gamma"]);
    second.serve(&["gamma", "alpha"]);
    first.asks("beta", "s-one", "p1");
    second.asks("gamma", "s-two", "p2");
    lab.route.shut.store(false, Ordering::Release);
    until("both newcomers have a card", || lab.cards().len() == 3);
    let (readers, cards) = (lab.readers(), lab.shown());
    let a = lab.follower("local/alpha~874493ab");
    let b = lab.follower("local/alpha~874493ab~2");
    assert_eq!(
        a.bound().as_deref(),
        Some(one),
        "the lesser instance id takes the plain hash"
    );
    assert_eq!(b.bound().as_deref(), Some(two));
    // Stable through later looks.
    lab.looks(12);
    assert_eq!(lab.readers(), readers);
    assert_eq!(lab.shown(), cards);
    assert!(Arc::ptr_eq(&a, &lab.follower("local/alpha~874493ab")));
    assert!(Arc::ptr_eq(&b, &lab.follower("local/alpha~874493ab~2")));
    // One ends: its gateway restarts as another instance. The other is
    // not renamed, not remade, and keeps its card.
    first.kill();
    let _again = Fake::serving(&lab.home, "one-again", &["alpha", "beta"]);
    until("the first newcomer's follower ended", || !a.active());
    lab.looks(12);
    assert!(Arc::ptr_eq(&b, &lab.follower("local/alpha~874493ab~2")) && b.active());
    assert_eq!(
        lab.observer.as_ref().unwrap().ends(),
        ["local/alpha~874493ab"]
    );
    let after = lab.shown();
    assert!(after
        .iter()
        .any(|id| id == "hermes:local/alpha~874493ab~2:gamma:s-two:p2"));
    eprintln!(
        "NAMES backwards={backwards} readers={readers:?} cards={cards:?}\n  after one ended: readers={:?} cards={after:?} ends={:?}",
        lab.readers(),
        lab.observer.as_ref().unwrap().ends()
    );
    (readers, cards)
}

#[test]
fn round4_the_name_fallback_is_the_same_whichever_home_is_asked_first_and_stays() {
    let forwards = collide(false);
    let backwards = collide(true);
    assert_eq!(forwards, backwards);
    assert_eq!(
        forwards.0,
        [
            "local",
            "local/alpha",
            "local/alpha~874493ab",
            "local/alpha~874493ab~2"
        ]
    );
}

/// The route of the tests, whose homes the look at the machine cannot be
/// told while `dark` is set. The followers are told as ever.
struct Dark {
    way: Arc<Way>,
    dark: AtomicBool,
    refused: AtomicBool,
}

impl Route for Dark {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        let looking = std::thread::current().name() == Some("Hermes gateways");
        if looking && self.dark.load(Ordering::Acquire) {
            self.refused.store(true, Ordering::Release);
            // Counted, so `Lab::looks` goes on counting looks.
            self.way.asked().homes += 1;
            return Err(Silent::Unclear);
        }
        self.way.homes()
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
}

/// The gate of a follower that is back from gone. It holds the polls for
/// as long as no look can be taken (the homes cannot be told), lets go
/// as soon as one settles, and never holds a stop.
#[test]
fn round4_the_gate_of_a_returning_follower_holds_while_no_look_settles_and_never_holds_a_stop() {
    let mut lab = lab();
    let route = Arc::new(Dark {
        way: lab.route.clone(),
        dark: AtomicBool::new(false),
        refused: AtomicBool::new(false),
    });
    let first = Fake::serving(&lab.home, "aaaaaaaa-first", &["default"]);
    let second = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    first.asks("default", "s-default", "p2");
    second.asks("beta", "s-beta", "p3");
    let pace = Pace {
        discover: Some(Duration::from_millis(40)),
        ..quick()
    };
    let wake = Arc::new(|| {});
    let observer = Observer::start_on(route.clone(), &lab.data, lab.requests.clone(), wake, pace);
    lab.observer = Some(observer.unwrap());
    until("both cards", || lab.cards().len() == 2);
    let follower = lab.follower("local/beta");
    second.kill();
    until("its follower knows it is gone", || {
        follower.gone.load(Ordering::Acquire)
    });
    // From here on no look at the machine can be taken.
    route.dark.store(true, Ordering::Release);
    until("a look was refused", || {
        route.refused.load(Ordering::Acquire)
    });
    let again = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    again.asks("beta", "s-returned", "p5");
    until("the follower has a socket on the gateway again", || {
        again.world().sockets > 0
    });
    let held = Instant::now();
    looks_for(&lab, Duration::from_millis(1500));
    let polls = again.world().calls("session.active_list").len();
    let shown = lab.shown();
    eprintln!(
        "GATE held_for={:?} polls_while_no_look_settles={polls} gone={} awaits={} settled={} cards={shown:?}",
        held.elapsed(),
        follower.gone.load(Ordering::Acquire),
        follower.awaits.load(Ordering::Acquire),
        follower.look.settled(),
    );
    // A look settles: the polls go on at once.
    route.dark.store(false, Ordering::Release);
    let freed = Instant::now();
    until("the returned gateway's question shows", || {
        lab.shown().iter().any(|id| id.contains("s-returned"))
    });
    eprintln!(
        "GATE freed_after={:?} cards={:?}",
        freed.elapsed(),
        lab.shown()
    );
    // Gone and back again, held again, and stopped while held.
    again.kill();
    until("gone again", || follower.gone.load(Ordering::Acquire));
    route.dark.store(true, Ordering::Release);
    lab.looks(2);
    let third = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    until("a socket again", || third.world().sockets > 0);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(third.world().calls("session.active_list").len(), 0);
    let stop = Instant::now();
    lab.observer.take();
    let stopped = stop.elapsed();
    until("the held follower's threads have exited", || {
        Arc::strong_count(&follower) == 1
    });
    eprintln!(
        "GATE stop_took={stopped:?} threads_exited_after={:?}",
        stop.elapsed()
    );
    assert!(
        stopped < Duration::from_millis(500),
        "a stop is not held by the gate"
    );
    assert_eq!(
        polls, 0,
        "control: the gate held the polls while no look settled"
    );
}

/// Unclear keeps the claim: the claimant's own home cannot be entered
/// (EACCES on the way to its socket), its follower loses its socket, and
/// the gateway is not taken for gone.
#[test]
fn round4_a_claimant_whose_home_cannot_be_entered_keeps_its_claim() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    let beta = home_of(&lab.home, "beta");
    std::fs::set_permissions(&beta, std::fs::Permissions::from_mode(0o000)).unwrap();
    let home = (lab.route.homes().unwrap().into_iter())
        .find(|home| home.profile().name() == "beta")
        .expect("the home is still told of");
    let said = control::ask(
        &*lab.route,
        &home,
        &identify_line(),
        Duration::from_millis(300),
    );
    second.cut();
    looks_for(&lab, Duration::from_secs(2));
    let gone = follower.gone.load(Ordering::Acquire);
    let (cards, contests) = (lab.shown(), lab.said("contested"));
    std::fs::set_permissions(&beta, std::fs::Permissions::from_mode(0o700)).unwrap();
    eprintln!("UNCLEAR_HOME said={said:?} gone={gone} cards={cards:?} contests={contests:?}");
    assert_eq!(said, Err(Silent::Unclear));
    assert!(!gone);
    assert_eq!(contests, ["contested profiles=[alpha]"]);
    assert!(!cards.iter().any(|id| id.contains("alpha:s-alpha:p1")));
}

/// Unclear keeps the claim: a listener that takes the connection and
/// drops it without a word.
#[test]
fn round4_a_claimant_that_takes_the_connection_and_drops_it_keeps_its_claim() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    let beta = home_of(&lab.home, "beta");
    let path = beta.join("gateway.sock");
    std::fs::remove_file(&path).unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let alive = running.clone();
    let worker = std::thread::spawn(move || {
        let mut taken = 0;
        while alive.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    taken += 1;
                    drop(stream);
                }
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        taken
    });
    let route = Local::at(&beta);
    let home = route.homes().unwrap().remove(0);
    let said = control::ask(&route, &home, &identify_line(), Duration::from_millis(300));
    second.cut();
    looks_for(&lab, Duration::from_secs(2));
    let gone = follower.gone.load(Ordering::Acquire);
    let (cards, contests) = (lab.shown(), lab.said("contested"));
    running.store(false, Ordering::Release);
    let taken = worker.join().unwrap();
    eprintln!("DROPPING_LISTENER said={said:?} taken={taken} gone={gone} cards={cards:?} contests={contests:?}");
    assert!(taken > 1);
    assert_ne!(said, Err(Silent::Absent));
    assert!(!gone);
    assert_eq!(contests, ["contested profiles=[alpha]"]);
    assert!(!cards.iter().any(|id| id.contains("alpha:s-alpha:p1")));
}

/// The remaining hold with no bound. A profile folder that has no
/// gateway at all and cannot be entered now answers Unclear at every
/// look, so no look is whole and no contest on the machine ever ends:
/// the claimant here lets alpha go by its own verified answer, and the
/// card of the gateway that serves alpha alone still never opens.
#[test]
fn round4_an_unrelated_folder_that_cannot_be_entered_ends_no_contest() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let locked = home_of(&lab.home, "locked");
    crate::os::private_directory(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let home = (lab.route.homes().unwrap().into_iter())
        .find(|home| home.profile().name() == "locked")
        .expect("the folder is told of as a home");
    let said = control::ask(
        &*lab.route,
        &home,
        &identify_line(),
        Duration::from_millis(300),
    );
    // Every look from here on has the folder among its homes.
    lab.looks(3);
    // A verified answer of the same instance: it serves alpha no more.
    second.serve(&["beta"]);
    looks_for(&lab, Duration::from_secs(2));
    let (cards, contests) = (lab.shown(), lab.said("contested"));
    let retired = lab.said("retired");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    eprintln!(
        "LOCKED_FOLDER said={said:?} retired={retired:?} cards={cards:?} contests={contests:?}\n{}",
        lab.log()
    );
    assert!(
        cards.iter().any(|id| id.contains("alpha:s-alpha:p1")),
        "a folder with no gateway must not keep a contest that its claimants have settled"
    );
}

// ---- Not the verifier's: what the first fix rests on, looked at alone ----

/// Only a home list that was read whole says that a home is not there.
#[test]
fn the_local_route_says_whether_it_read_every_home() {
    let lab = lab();
    let route = Local::at(&lab.home);
    let told = |route: &Local| {
        let told = route.inventory().unwrap();
        let name = |home: &Home| home.profile().name().to_owned();
        (told.homes.iter().map(name).collect::<Vec<_>>(), told.whole)
    };
    // No `profiles` at all: the root is every home there is.
    assert_eq!(told(&route), (vec!["default".to_owned()], true));
    let profiles = lab.home.join("profiles");
    for name in ["beta", "alpha"] {
        crate::os::private_directory(&home_of(&lab.home, name)).unwrap();
    }
    // A file, and a folder Hermes would name no profile by, are no homes.
    std::fs::write(profiles.join("notes"), b"").unwrap();
    crate::os::private_directory(&profiles.join("Not A Profile")).unwrap();
    let all = vec!["default".to_owned(), "alpha".to_owned(), "beta".to_owned()];
    assert_eq!(told(&route), (all.clone(), true));
    // A home that cannot be entered is still a home that is there.
    let mode = |path: &Path, bits| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap()
    };
    mode(&home_of(&lab.home, "beta"), 0o000);
    assert_eq!(told(&route), (all, true));
    mode(&home_of(&lab.home, "beta"), 0o700);
    // `profiles` cannot be opened: the root alone, and not all there is.
    mode(&profiles, 0o000);
    assert_eq!(told(&route), (vec!["default".to_owned()], false));
    assert_eq!(
        route.homes().unwrap().len(),
        1,
        "the root is told all the same"
    );
    mode(&profiles, 0o700);
    // A link that leads nowhere is a home that is not there.
    std::os::unix::fs::symlink(lab.home.join("nowhere"), profiles.join("gone")).unwrap();
    assert!(told(&route).1);
    // A route that only tells of homes says they are all there are.
    let dark = Dark {
        way: lab.route.clone(),
        dark: AtomicBool::new(false),
        refused: AtomicBool::new(false),
    };
    assert!(dark.inventory().unwrap().whole);
}
