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
