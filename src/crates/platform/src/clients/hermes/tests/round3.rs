//! The third review of ticket 26 (`evidence/26/verifier-report-3.md`):
//! its probes (`verifier-files-3/round3.rs` and `stop_retry.rs`), tracked
//! as tests. Five of them failed on `f7ee472`: two newcomers told apart
//! alike, a stopped gateway whose home was removed, a control socket its
//! owner may not use, the same instance back from gone, and a retry
//! begun after the stop. Two passed there and pass now: a slow control
//! listener is not gone, and a name told apart is safe to write.
//!
//! One line is changed, in the permission probe. It asserted, as a
//! control, that the route took a permission failure for absence, which
//! was the defect itself. It now asserts what the route says instead:
//! the socket is not the user's own to use. Every other assertion stands,
//! and the file is otherwise as the verifier wrote it, formatted.
use super::super::{
    control::{self, Silent},
    route::{local::Local, Route},
};
use super::{
    fake::{home_of, Fake, Hold},
    lab, until, Lab,
};
use agent_hud_agents::hermes::gateway::{identify_line, GatewayId, Machine, Profile};
use std::{
    io::{BufRead, BufReader},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn contest(lab: &mut Lab) -> (Fake, Fake) {
    contest_at(lab, Duration::from_millis(40))
}
fn contest_at(lab: &mut Lab, discover: Duration) -> (Fake, Fake) {
    let first = Fake::serving(&lab.home, "aaaaaaaa-first", &["default", "alpha"]);
    let second = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    second.serve(&["beta", "alpha"]);
    first.asks("default", "s-default", "p2");
    first.asks("alpha", "s-alpha", "p1");
    second.asks("beta", "s-beta", "p3");
    second.asks("alpha", "s-other", "p4");
    lab.follow_at(super::super::Pace {
        discover: Some(discover),
        ..super::quick()
    });
    until("contest and uncontested cards", || {
        lab.cards().len() == 2 && lab.said("contested") == ["contested profiles=[alpha]"]
    });
    lab.looks(4);
    (first, second)
}

#[test]
fn round3_hash_collision_cannot_hide_a_second_newcomer() {
    let mut lab = lab();
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
    let alpha = Profile::named("alpha").unwrap();
    let a = GatewayId::apart(Machine::This, alpha.clone(), one).to_string();
    let b = GatewayId::apart(Machine::This, alpha, two).to_string();
    assert_eq!(a, b, "control: a real SHA-256 prefix collision");
    let first = Fake::serving(&lab.home, one, &["alpha", "beta"]);
    first.asks("beta", "s-first", "p1");
    until("first newcomer followed", || {
        lab.shown().iter().any(|id| id.contains("beta:s-first:p1"))
    });
    let kept = lab.follower(&a);
    first.serve(&["beta"]);
    until("first relinquishes alpha", || {
        !home_of(&lab.home, "alpha").join("gateway.sock").exists()
    });
    lab.looks(8);
    let second = Fake::serving(&lab.home, two, &["alpha"]);
    second.asks("alpha", "s-second", "p2");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        lab.looks(5);
    }
    let polls = second.world().calls("session.active_list").len();
    eprintln!("HASH_COLLISION one={one} two={two} a={a} b={b} elapsed={:?} polls={polls} readers={:?} cards={:?}\n{}", start.elapsed(), lab.readers(), lab.shown(), lab.log());
    assert!(Arc::ptr_eq(&kept, &lab.follower(&a)) && kept.active());
    assert!(
        lab.shown()
            .iter()
            .any(|id| id.contains("alpha:s-second:p2")),
        "the second verified newcomer must have a follower and a card"
    );
}

#[test]
fn round3_same_instance_regains_claims_after_being_gone() {
    let mut lab = lab();
    let (_first, second) = contest_at(&mut lab, Duration::from_millis(250));
    let follower = lab.follower("local/alpha");
    second.kill();
    until("gone claimant releases contest", || {
        lab.shown().iter().any(|id| id.contains("alpha:s-alpha:p1"))
    });
    let again = Fake::serving(&lab.home, "bbbbbbbb-second", &["beta"]);
    again.serve(&["beta", "alpha"]);
    again.asks("alpha", "s-returned", "p5");
    until("same-instance claim returns", || {
        lab.said("contested").len() == 3
    });
    lab.looks(8);
    assert!(Arc::ptr_eq(&follower, &lab.follower("local/alpha")));
    assert!(follower.active() && !follower.gone.load(Ordering::Acquire));
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    eprintln!(
        "SAME_INSTANCE readers={:?} cards={:?} contests={:?}\n{}",
        lab.readers(),
        lab.shown(),
        lab.said("contested"),
        lab.log()
    );
    assert!(
        !lab.shown().iter().any(|id| id.contains("s-returned")),
        "a returning same-instance claimant must regain its contest before polling can open cards"
    );
    again.serve(&["beta"]);
    until("verified same instance relinquishes alpha", || {
        lab.said("contested").len() == 4
    });
    eprintln!(
        "SAME_INSTANCE readers={:?} cards={:?} contests={:?}",
        lab.readers(),
        lab.shown(),
        lab.said("contested")
    );
}

#[test]
fn round3_a_stopped_gateway_with_its_home_removed_releases_claims() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    lab.route.slow.lock().unwrap().push("beta".to_owned());
    second.kill();
    let beta = home_of(&lab.home, "beta");
    std::fs::remove_dir(&beta).unwrap();
    lab.route.slow.lock().unwrap().clear();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        lab.looks(5);
    }
    eprintln!(
        "REMOVED_HOME exists={} gone={} active={} cards={:?} contests={:?}\n{}",
        beta.exists(),
        follower.gone.load(Ordering::Acquire),
        follower.active(),
        lab.shown(),
        lab.said("contested"),
        lab.log()
    );
    assert!(follower.active());
    assert!(
        lab.shown().iter().any(|id| id.contains("alpha:s-alpha:p1")),
        "a gateway with no sockets and no home must not hold the other card back forever"
    );
}

struct Listener {
    running: Arc<AtomicBool>,
    accepted: Arc<AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn listener(home: &std::path::Path, reset: bool) -> Listener {
    let path = home.join("gateway.sock");
    std::fs::remove_file(&path).unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let accepted = Arc::new(AtomicUsize::new(0));
    let (r, a) = (running.clone(), accepted.clone());
    let worker = std::thread::spawn(move || {
        while r.load(Ordering::Acquire) {
            let Ok((stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            a.fetch_add(1, Ordering::AcqRel);
            if reset {
                // Let the request arrive, then close without consuming it.
                // The listener remains up and accepts subsequent connections.
                std::thread::sleep(Duration::from_millis(10));
                drop(stream);
            } else {
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    Listener {
        running,
        accepted,
        worker: Some(worker),
    }
}

#[test]
fn round3_a_private_control_socket_permission_error_is_not_gone() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    let beta = home_of(&lab.home, "beta");
    let path = beta.join("gateway.sock");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let raw_error = std::os::unix::net::UnixStream::connect(&path).unwrap_err();
    let route = Local::at(&beta);
    let home = route.homes().unwrap().remove(0);
    let result = control::ask(&route, &home, &identify_line(), Duration::from_millis(300));
    eprintln!(
        "CONTROL_PERMISSION raw={raw_error:?} result={result:?} socket_exists={}",
        path.exists()
    );
    assert_eq!(
        result,
        Err(Silent::Unsafe("socket")),
        "a permission failure is not absence"
    );
    second.cut();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        lab.looks(5);
    }
    let gone = follower.gone.load(Ordering::Acquire);
    let cards = lab.shown();
    eprintln!(
        "LIVE_LISTENER_PERMISSION gone={gone} cards={cards:?} contests={:?}\n{}",
        lab.said("contested"),
        lab.log()
    );
    // A successful connection after restoring permissions proves the
    // same listener remained up throughout the failure and claim release.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let answered = control::ask(&route, &home, &identify_line(), Duration::from_millis(300));
    assert!(answered.is_ok(), "the listener never stopped");
    assert!(
        !cards.iter().any(|id| id.contains("alpha:s-alpha:p1")),
        "permission denial is not proof nobody listens at the claimant's socket"
    );
}

#[test]
fn round3_a_slow_control_listener_is_not_gone() {
    let mut lab = lab();
    let (_first, second) = contest(&mut lab);
    let follower = lab.follower("local/alpha");
    let beta = home_of(&lab.home, "beta");
    let listener = listener(&beta, false);
    let route = Local::at(&beta);
    let home = route.homes().unwrap().remove(0);
    let result = control::ask(&route, &home, &identify_line(), Duration::from_millis(30));
    assert!(matches!(result, Err(Silent::TimedOut)));
    second.cut();
    lab.looks(8);
    eprintln!(
        "SLOW_CONTROL accepts={} gone={} cards={:?}",
        listener.accepted.load(Ordering::Acquire),
        follower.gone.load(Ordering::Acquire),
        lab.shown()
    );
    assert!(!follower.gone.load(Ordering::Acquire));
    assert_eq!(lab.said("contested"), ["contested profiles=[alpha]"]);
}

#[test]
fn round3_disambiguated_ids_are_stable_and_path_safe() {
    for anchor in ["a", "z-9_test", &"a".repeat(64)] {
        for instance in ["plain", "../../bad\n:~/\\", "é🐈"] {
            let profile = Profile::named(anchor).unwrap();
            let id = GatewayId::apart(Machine::This, profile.clone(), instance).to_string();
            assert_eq!(
                id,
                GatewayId::apart(Machine::This, profile.clone(), instance).to_string()
            );
            assert_ne!(id, GatewayId::new(Machine::This, profile).to_string());
            assert!(id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/_-~".contains(&c)));
            assert_eq!(std::path::Path::new(&id).components().count(), 2);
            let last = id.split('/').nth(1).unwrap();
            assert!(Profile::named(last).is_none());
            assert_eq!(last.split('~').nth(1).unwrap().len(), 8);
        }
    }
}

#[test]
fn round3_stop_must_not_begin_a_second_identify_after_a_held_timeout() {
    let mut lab = lab();
    let held = Arc::new(Hold::default());
    *lab.route.held.lock().unwrap() = Some(("exchange", held.clone()));
    lab.route.slow.lock().unwrap().push("default".to_owned());
    lab.follow();
    until("the first identify is held", || {
        held.entered.load(Ordering::Acquire)
    });
    let follower = lab.observer.as_ref().unwrap().local();
    lab.observer.take();
    let before = lab.route.asked().verbs.clone();
    held.released.store(true, Ordering::Release);
    until("the follower threads have exited", || {
        Arc::strong_count(&follower) == 1
    });
    let after = lab.route.asked().verbs.clone();
    eprintln!(
        "STOP_IDENTIFY before={before:?} after={after:?} active={} log=\n{}",
        follower.active(),
        lab.log()
    );
    assert_eq!(before, ["identify"]);
    assert_eq!(
        after, before,
        "a timeout returned after stop must not start another identify"
    );
}

// ---- Not the verifier's: what the fixes rest on, each looked at alone ----

/// Only two errors on the way to a listener say that nobody listens.
#[test]
fn only_a_missing_socket_and_a_refused_connection_say_nobody_listens() {
    use super::super::route::local::nobody;
    use std::io::{Error, ErrorKind};
    for kind in [ErrorKind::NotFound, ErrorKind::ConnectionRefused] {
        assert_eq!(nobody(&Error::from(kind)), Silent::Absent, "{kind:?}");
    }
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::ConnectionReset,
        ErrorKind::BrokenPipe,
        ErrorKind::Interrupted,
        ErrorKind::Other,
    ] {
        assert_eq!(nobody(&Error::from(kind)), Silent::Unclear, "{kind:?}");
    }
}

/// A control socket is asked only where its owner may read and write it,
/// and no one else may do either.
#[test]
fn a_control_socket_its_owner_may_not_read_and_write_is_not_asked() {
    let lab = lab();
    let _gateway = Fake::serving(&lab.home, "aaaaaaaa-one", &["default"]);
    let path = lab.home.join("gateway.sock");
    let route = Local::at(&lab.home);
    let home = route.homes().unwrap().remove(0);
    let ask = || control::ask(&route, &home, &identify_line(), Duration::from_secs(2));
    for mode in [0o000, 0o200, 0o400, 0o500, 0o660, 0o606] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(ask(), Err(Silent::Unsafe("socket")), "{mode:o}");
    }
    for mode in [0o600, 0o700] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(ask().is_ok(), "{mode:o}");
    }
    // A listener that takes the connection and drops it was there.
    let dropped = listener(&lab.home, true);
    let reset = ask();
    assert!(dropped.accepted.load(Ordering::Acquire) > 0);
    assert_ne!(reset, Err(Silent::Absent), "somebody listened");
}
