//! The relay's own deadline: it is gone when it passes, whatever the
//! gateway does and whatever became of the `ssh` client that carried it.
use super::{
    super::{Far, Silent},
    fixture::{chmod, text, FarLab, Manner, SHORT},
};
use crate::ssh_link::unix::tests::lab::until;
use agent_hud_agents::hermes::gateway::far::{Quiet, Request};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant, UNIX_EPOCH},
};

/// The far `python3` is a stand-in that runs the relay in its own process
/// and notes, by the wall clock, the moment the relay's code begins: after
/// Python has started, read the program, compiled it and imported what it
/// imports, which a busy machine makes slow and which comes before any
/// deadline can be armed (ticket 48).
const STAMPING: &str = r#"#!/bin/sh
exec /usr/bin/python3 -I -S -B -c 'import sys, time
import errno, fcntl, hashlib, json, os, signal, socket, stat, struct, threading
stamps, rest = sys.argv[1], sys.argv[2:]
sys.argv = ["-"] + rest[rest.index("-") + 1:]
code = compile(sys.stdin.read(), "<stdin>", "exec")
open(stamps, "a").write("%r\n" % time.time())
exec(code, {"__name__": "__main__"})' '{stamps}' "$@"
"#;

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_silent_gateway_ends_at_the_deadline() {
    let path = ("PATH", "{lab}/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    let Some(lab) = FarLab::start_with(&[path]) else {
        return;
    };
    let stamps = lab.dir.join("relay-starts");
    fs::create_dir_all(lab.dir.join("bin")).expect("the stand-in's folder");
    let stand_in = lab.dir.join("bin").join("python3");
    let script = STAMPING.replace("{stamps}", &stamps.display().to_string());
    fs::write(&stand_in, script).expect("the stand-in");
    chmod(&stand_in, 0o755);
    // When the latest relay's own code began.
    let began = || {
        let noted = fs::read_to_string(&stamps).expect("a relay began");
        let last: f64 = noted.lines().last().unwrap().parse().unwrap();
        UNIX_EPOCH + Duration::from_secs_f64(last)
    };
    // This side waits long enough that only the relay's own deadline can
    // end the exchange, however long a busy machine takes to start
    // `python3` over `ssh` (ticket 48: past the 2 s margin under load,
    // which gave `CutOff`).
    let far = Far::new(SHORT).waiting_longer(Duration::from_secs(60));
    let deadline = Duration::from_secs(SHORT.into());
    // The relay's deadline counts from its start: it lives no longer than
    // that, and a little, from its own code beginning to its end of the
    // socket going. Both are taken on the far side, so neither `ssh` nor
    // Python's own start is in it.
    let within = deadline + Duration::from_millis(1900);
    for (name, manner) in [
        ("never answers", Manner::Silent),
        ("never ends", Manner::Drips),
    ] {
        let home = lab.profile(if name == "never answers" { "a" } else { "b" });
        let gate = lab.gate(&home, manner);
        let started = Instant::now();
        let told = far.exchange(&lab.link, text(&home), Request::Identify);
        let waited = started.elapsed();
        // The gateway's own thread notes it, however late it runs.
        assert!(until(Duration::from_secs(10), || gate.held().len() == 1));
        let lived = gate.gone_at()[0]
            .duration_since(began())
            .unwrap_or_default();
        let held = gate.held();
        println!("a gateway that {name}: the relay ended after {waited:?}, lived {lived:?}, held the line {held:?}");
        // Status 3: the relay ended itself at its deadline.
        assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::TimedOut));
        assert!(waited >= deadline, "{waited:?}");
        assert!(lived < within, "lived {lived:?}");
        assert_eq!(lab.relays(&home), 0, "the far relay is gone");
        assert_eq!(gate.seen().len(), 1);
        // The relay ended, so the home is not counted as busy.
        let again = far.exchange(&lab.link, text(&home), Request::Identify);
        assert_eq!(again.said.unwrap_err(), Silent::Relay(Quiet::TimedOut));
        assert_eq!(gate.seen().len(), 2);
        assert!(until(Duration::from_secs(10), || gate.held().len() == 2));
        let lived = gate.gone_at()[1]
            .duration_since(began())
            .unwrap_or_default();
        assert!(lived < within, "lived {lived:?}");
    }
}

/// The `ssh` clients of this lab that carry a relay for `home`.
fn carriers(lab: &FarLab, home: &str) -> Vec<i32> {
    let listed = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains("ControlPath") && line.contains(" -I -S -B - "))
        .filter(|line| line.contains(home) && line.contains(text(&lab.lab.at(""))))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_cut_off_relay_is_gone_by_its_deadline() {
    let Some(lab) = FarLab::start() else { return };
    let seconds = 5;
    let far = Far::new(seconds);
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Silent);
    let path = text(&home).to_owned();
    let (told, cut) = std::thread::scope(|scope| {
        let exchange = scope.spawn(|| far.exchange(&lab.link, &path, Request::Identify));
        // Mid-exchange: the gateway has the line and says nothing.
        assert!(until(Duration::from_secs(10), || gate.seen().len() == 1));
        let carriers = carriers(&lab, &path);
        assert_eq!(carriers.len(), 1, "the one ssh client that carries it");
        // SAFETY: a signal to the `ssh` client this test's own link started.
        unsafe { libc::kill(carriers[0], libc::SIGKILL) };
        let cut = Instant::now();
        (exchange.join().unwrap(), cut)
    });
    println!("the exchange came back {:?} after the kill", cut.elapsed());
    assert_eq!(told.said.unwrap_err(), Silent::CutOff);
    // The documented limit: the far relay outlives the client that carried it.
    assert_eq!(lab.relays(&home), 1, "the far relay runs on");
    // So no second exchange is started with that home before its deadline.
    let second = far.exchange(&lab.link, &path, Request::Identify);
    assert_eq!(second.said.unwrap_err(), Silent::Busy);
    assert_eq!(gate.connections(), 1, "nothing was started");
    // Another home of the same machine is not held up.
    let other = lab.profile("ops");
    let _other = lab.gate(&other, Manner::gateway("two", &[&other]));
    assert!(far
        .exchange(&lab.link, text(&other), Request::Identify)
        .said
        .is_ok());
    // At its deadline the far relay is gone.
    let deadline = Duration::from_secs(seconds.into());
    assert!(until(deadline + Duration::from_secs(1), || lab
        .relays(&home)
        == 0));
    let gone = cut.elapsed();
    println!("the far relay was gone {gone:?} after the kill");
    assert!(gone <= deadline + Duration::from_secs(1), "{gone:?}");
    assert_eq!(gate.connections(), 1);
    // Still held back, until the deadline has surely passed.
    if cut.elapsed() < deadline {
        let held = far.exchange(&lab.link, &path, Request::Identify);
        assert_eq!(held.said.unwrap_err(), Silent::Busy);
    }
    // Then the home is asked again.
    assert!(until(Duration::from_secs(10), || {
        far.exchange(&lab.link, &path, Request::Identify).said != Err(Silent::Busy)
    }));
    assert!(cut.elapsed() >= deadline, "not before the deadline");
    assert!(gate.connected(2));
    assert!(until(Duration::from_secs(10), || lab.relays(&home) == 0));
}
