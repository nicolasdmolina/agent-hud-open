//! A link that died says so, with why, within the keepalive bound.
use super::lab::{entries, far, until, Lab};
use crate::ssh_link::{End, Failure, Link, Lost};
use std::{
    fs,
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_killed_master_is_lost_within_the_keepalive_bound_and_a_new_link_works() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = lab.settings();
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    assert_eq!(link.alive(), Ok(()));
    let pid = link.master_pid().unwrap();
    // SAFETY: a signal to the master this test just started.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    let killed = Instant::now();
    assert!(until(settings.keepalive_bound(), || link.alive().is_err()));
    println!("reported lost {:?} after the kill", killed.elapsed());
    assert!(killed.elapsed() < settings.keepalive_bound());
    assert_eq!(link.alive(), Err(Lost::Ended));
    assert_eq!(
        link.run(&far("true", &[])).unwrap_err(),
        Failure::Lost(Lost::Ended)
    );
    // The kill left the socket behind; closing the link removes it.
    assert!(link.close().gone);
    assert_eq!(entries(folder.path()), Vec::<String>::new());

    let again = Link::open(&folder, &lab.target(), &settings).expect("a new link opens");
    assert_eq!(again.run(&far("exit 3", &[])).unwrap().end, End::Ended(3));
    assert!(again.close().gone);
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_silent_far_side_is_lost_as_timed_out_within_the_keepalive_bound() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = lab.settings();
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    lab.freeze();
    let silent = Instant::now();
    // The bound is two keepalives a second apart; the rest is the time
    // this loaded machine takes to notice.
    let allowed = settings.keepalive_bound() + Duration::from_secs(6);
    assert!(until(allowed, || link.alive().is_err()));
    println!(
        "reported lost {:?} after the far side went silent",
        silent.elapsed()
    );
    assert_eq!(link.alive(), Err(Lost::TimedOut));
    assert_eq!(Lost::TimedOut.reason(), "timed out");
    assert!(link.close().gone);
    assert_eq!(entries(folder.path()), Vec::<String>::new());
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
}

/// A master that stays up for weeks writes a line for every connection a
/// forward could not make. Its log is emptied once it is long, and what it
/// writes afterwards, its last words among it, starts the file again.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_long_log_is_emptied_and_the_masters_last_words_are_still_read() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = lab.settings();
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    let log = entries(folder.path())
        .into_iter()
        .find(|name| name.ends_with(".log"))
        .map(|name| folder.path().join(name))
        .unwrap();
    // A forward with no far listener makes the master write a line.
    let nobody = SocketAddr::from((Ipv4Addr::LOCALHOST, super::lab::closed_port()));
    let forward = link.forward(nobody).unwrap();
    assert!(until(Duration::from_secs(10), || {
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, forward.local));
        fs::metadata(&log).is_ok_and(|node| node.len() > 0)
    }));
    let mut filler = fs::OpenOptions::new().append(true).open(&log).unwrap();
    filler.write_all(&vec![b'x'; 300 * 1024]).unwrap();
    assert_eq!(link.alive(), Ok(()));
    assert_eq!(fs::metadata(&log).unwrap().len(), 0);

    lab.freeze();
    let allowed = settings.keepalive_bound() + Duration::from_secs(6);
    assert!(until(allowed, || fs::metadata(&log)
        .is_ok_and(|node| node.len() > 0)));
    let said = fs::read_to_string(&log).unwrap();
    assert!(said.starts_with("Timeout, server"), "{said:?}");
    assert!(until(allowed, || link.alive().is_err()));
    assert_eq!(link.alive(), Err(Lost::TimedOut));
    link.close();
}

/// A master that was killed shows as gone at once, by a look at its
/// process that starts none (ticket 30).
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_killed_master_shows_as_not_running_without_a_process_being_started() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    assert_eq!(link.running(), Ok(()));
    let before = lab.link_processes(&folder);
    assert_eq!(link.running(), Ok(()));
    assert_eq!(
        lab.link_processes(&folder),
        before,
        "the master and no client"
    );
    let pid = link.master_pid().unwrap();
    // SAFETY: a signal to the master this test just started.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    let killed = Instant::now();
    assert!(until(Duration::from_secs(2), || link.running().is_err()));
    println!("seen as not running {:?} after the kill", killed.elapsed());
    assert_eq!(link.running(), Err(Lost::Ended));
    assert!(link.close().gone);
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}

/// An opening can be given up while the master still signs in: the master
/// is ended, and nothing of it is left (ticket 30).
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn an_opening_that_is_given_up_ends_its_master_and_leaves_nothing() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    // A proxy that never connects: the master waits on it.
    let stuck = lab.config_text().replace(
        &format!("Port {}", lab.port),
        "Port 9\n  ProxyCommand /bin/sleep 600",
    );
    let settings = lab.settings_with("stuck", &stuck);
    let wanted = AtomicBool::new(true);
    let began = Instant::now();
    let opened = std::thread::scope(|scope| {
        let opening = scope.spawn(|| {
            let still = || wanted.load(Ordering::Acquire);
            Link::open_while(&folder, &lab.target(), &settings, &still)
        });
        assert!(until(Duration::from_secs(10), || {
            lab.link_processes(&folder).len() == 1
        }));
        wanted.store(false, Ordering::Release);
        opening.join().expect("the opening ends")
    });
    println!("given up after {:?}", began.elapsed());
    assert_eq!(
        opened.err(),
        Some(Failure::Refused(crate::ssh_link::Refused::Closed))
    );
    assert!(began.elapsed() < settings.connect_timeout);
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
    // An opening that stays wanted opens as ever.
    assert!(Link::open_while(&folder, &lab.target(), &lab.settings(), &|| true).is_ok());
}
