//! The forwards on real links: two masters to the throwaway `sshd` on
//! loopback (`ssh_link/unix/tests/lab.rs`), so what listens on this
//! machine is counted from the masters' own open sockets and not from
//! what the table believes. Unix only, as the lab is.
//!
//! The test starts processes, so it is marked `#[ignore]` and run by
//! `the_lab_tests_pass` in a process of its own. To run it here and see
//! what it prints:
//! `cargo test -p agent-hud-platform --lib clients::hermes::route::ssh::forwards::tests_unix:: -- --ignored --nocapture`.
use super::{
    super::{super::Stop, tests::accepted},
    Forwards,
};
use crate::ssh_link::{
    unix::tests::lab::{self, Lab},
    Link,
};
use agent_hud_agents::hermes::gateway::Failure;
use std::{process::Command, sync::Arc, time::Duration};

/// How long a cancel may take to show in what the master listens on: it
/// is asked of the master on a thread of its own.
const CANCEL: Duration = Duration::from_secs(10);

#[test]
fn the_lab_tests_pass() {
    lab::in_a_process_of_their_own("clients::hermes::route::ssh::forwards::tests_unix::");
}

/// Whoever asks for a forward and never stops.
struct Never;

impl Stop for Never {
    fn wait(&self, _: Duration) -> bool {
        false
    }
}

/// Where the master of `link` listens on this machine.
fn listeners(link: &Link) -> Vec<String> {
    let pid = link.master_pid().unwrap().to_string();
    let output = Command::new("lsof")
        .args(["-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fn"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::to_owned)
        .collect()
}

/// Where the master of `link` listens, once that is nowhere or `CANCEL`
/// has passed.
fn listeners_after_a_cancel(link: &Link) -> Vec<String> {
    lab::until(CANCEL, || listeners(link).is_empty());
    listeners(link)
}

/// The third verifier's probe of ticket 30 (`evidence/30/verifier3-files/
/// probes/forwards-probe.rs`), its statements as written. A socket that
/// took the link just before it was lost reaches `prepare` after another
/// socket has prepared on the link that replaced it. It failed on the
/// reviewed head: the late socket's older number cleared the table of
/// the open link, so the next socket made a second forward beside the
/// first, which nothing could cancel any more (ticket 50).
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_prepare_that_holds_an_older_link_number_orphans_no_forward() {
    let Some(lab) = Lab::start() else {
        return;
    };
    let open =
        || Arc::new(Link::open(&lab.folder(), &lab.target(), &lab.settings()).expect("a link"));
    let (old, new) = (open(), open());
    let forwards = Arc::new(Forwards::default());
    let instance = accepted("http://127.0.0.1:8642").unwrap();
    // The polls' socket prepared on the link that is open now, number 2.
    forwards
        .prepare(&new, 2, &instance, 8642, &Never)
        .expect("the forward on link 2");
    let first = forwards.local(2, &instance.id);
    assert_eq!(listeners(&new).len(), 1);
    // Link 1 is gone. A visit that read `keeper.link()` while it was
    // still the open one arrives now.
    old.close();
    let late = forwards.prepare(&old, 1, &instance, 8642, &Never);
    let after_late = forwards.local(2, &instance.id);
    // The next socket on link 2.
    let again = forwards.prepare(&new, 2, &instance, 8642, &Never);
    let second = forwards.local(2, &instance.id);
    let now = listeners(&new);
    let made = forwards.made();
    println!(
        "TRACE stale link number late={late:?} first_local={first:?} local_after_late={after_late:?} \
         again={again:?} second_local={second:?} listeners_on_link_2={now:?} made={made}"
    );
    let count = now.len();
    new.close();
    assert_eq!(count, 1, "a forward on the open link was orphaned");
    // The late socket is told there is no way through, and changed nothing.
    assert_eq!(late, Err(Failure::Unreachable));
    assert!(first.is_some());
    assert_eq!(after_late, first, "the open link's forward was forgotten");
    assert_eq!((again, second), (Ok(()), first));
    assert_eq!(made, 1, "one gateway, one forward");
}

/// The verifier's probe B of ticket 50 (`evidence/50/verifier-files/probes/
/// forwards-probes.rs`), its statements as written, with each fixed wait
/// of two seconds made a wait for the cancel. A follower that ended read
/// `keeper.link()` while link 1 was open, and its `release` arrives after
/// another socket prepared on link 2. It failed on ticket 50's merge: the
/// forward was taken out of the table and owed, and what is owed was paid
/// only to whoever held the table's own number, so it listened on until
/// the next forward or release on link 2 (ticket 51). The same is held
/// here for every gateway at once (`release_all`), and for a release that
/// read the keeper while no link was open.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_release_that_holds_an_older_link_number_leaves_no_forward_listening() {
    let Some(lab) = Lab::start() else {
        return;
    };
    let open =
        || Arc::new(Link::open(&lab.folder(), &lab.target(), &lab.settings()).expect("a link"));
    let (old, new) = (open(), open());
    let instance = accepted("http://127.0.0.1:8642").unwrap();
    let prepare_on = |all: &Arc<Forwards>, link: &Arc<Link>, number: u64| {
        all.prepare(link, number, &instance, 8642, &Never)
    };
    let prepare = |all: &Arc<Forwards>| prepare_on(all, &new, 2);
    // Control: a release with the open link's number cancels the forward.
    let control = Arc::new(Forwards::default());
    prepare(&control).expect("control forward");
    let control_before = listeners(&new).len();
    control.release(Some((new.clone(), 2)), &instance.id);
    let control_after = listeners_after_a_cancel(&new).len();
    // The stale release.
    let forwards = Arc::new(Forwards::default());
    prepare(&forwards).expect("the forward on link 2");
    let before = listeners(&new).len();
    old.close();
    // A socket of link 1 that comes for a forward is refused, and the
    // table's link stays the open one for the release after it.
    let late = prepare_on(&forwards, &old, 1);
    forwards.release(Some((old.clone(), 1)), &instance.id);
    let after = listeners_after_a_cancel(&new);
    let local = forwards.local(2, &instance.id);
    println!(
        "TRACE stale release control_before={control_before} control_after={control_after} \
         before={before} listeners_on_link_2_after={after:?} local_after={local:?}"
    );
    // The machine is followed no more, as told by one that holds link 1.
    prepare(&forwards).expect("the second forward on link 2");
    let all_before = listeners(&new).len();
    forwards.release_all(Some((old.clone(), 1)));
    let all_after = listeners_after_a_cancel(&new);
    // A follower that ended while no link was open, and arrives now.
    prepare(&forwards).expect("the third forward on link 2");
    let none_before = listeners(&new).len();
    forwards.release(None, &instance.id);
    let none_after = listeners_after_a_cancel(&new);
    let made = forwards.made();
    println!(
        "TRACE stale release_all before={all_before} after={all_after:?}; no link \
         before={none_before} after={none_after:?} made={made}"
    );
    new.close();
    assert_eq!(
        (control_before, control_after),
        (1, 0),
        "the control did not cancel"
    );
    assert_eq!(late, Err(Failure::Unreachable));
    let still = "the forward of an instance followed no more still listens";
    assert_eq!((before, after.len(), local), (1, 0, None), "{still}");
    assert_eq!((all_before, all_after.len()), (1, 0), "{still}");
    assert_eq!((none_before, none_after.len()), (1, 0), "{still}");
    assert_eq!(made, 3, "one forward for each time it was prepared");
}
