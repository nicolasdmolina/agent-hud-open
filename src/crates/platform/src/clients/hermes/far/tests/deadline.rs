//! The relay's own deadline: it is gone when it passes, whatever the
//! gateway does and whatever became of the `ssh` client that carried it.
use super::{
    super::{Far, Silent},
    fixture::{text, FarLab, Manner, SHORT},
};
use crate::ssh_link::unix::tests::lab::until;
use agent_hud_agents::hermes::gateway::far::{Quiet, Request};
use std::{
    process::Command,
    time::{Duration, Instant},
};

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_silent_gateway_ends_at_the_deadline() {
    let Some(lab) = FarLab::start() else { return };
    let far = Far::new(SHORT);
    let deadline = Duration::from_secs(SHORT.into());
    for (name, manner) in [
        ("never answers", Manner::Silent),
        ("never ends", Manner::Drips),
    ] {
        let home = lab.profile(if name == "never answers" { "a" } else { "b" });
        let gate = lab.gate(&home, manner);
        let started = Instant::now();
        let told = far.exchange(&lab.link, text(&home), Request::Identify);
        let waited = started.elapsed();
        println!("a gateway that {name}: the relay ended after {waited:?}");
        assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::TimedOut));
        // It ended at its own deadline, before this side's longer wait.
        assert!(waited >= deadline, "{waited:?}");
        assert!(
            waited < deadline + Duration::from_millis(1900),
            "{waited:?}"
        );
        assert_eq!(lab.relays(&home), 0, "the far relay is gone");
        assert_eq!(gate.seen().len(), 1);
        // The relay ended, so the home is not counted as busy.
        let again = far.exchange(&lab.link, text(&home), Request::Identify);
        assert_eq!(again.said.unwrap_err(), Silent::Relay(Quiet::TimedOut));
        assert_eq!(gate.seen().len(), 2);
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
