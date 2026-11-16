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
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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

/// The far `python3` for the cut-off relay: a parent that starts the
/// relay, waits for it and notes by the wall clock when its process ended,
/// with its arguments. It takes them from a file, so the far process list
/// shows one relay, as before. The relay notes when its own code begins,
/// as `STAMPING` does. Both moments are the far side's own, so how slowly a
/// busy machine lists its processes is in neither (ticket 49).
const LIFE: &str = r#"import os, subprocess, sys, time
given, begins, stamps, ends = sys.argv[1:5]
args = open(given).read().split("\n")[:-1]
relay = subprocess.Popen(["/usr/bin/python3", "-I", "-S", "-B", "-c", open(begins).read(), stamps] + args)
os.close(0)
os.close(1)
status = os.waitpid(relay.pid, 0)[1]
open(ends, "a").write("%d %r %s\n" % (relay.pid, time.time(), " ".join(args)))
sys.exit(os.waitstatus_to_exitcode(status) % 256)
"#;

/// What the relay runs first: `STAMPING`'s code, which also notes its
/// process id.
const BEGINS: &str = r#"import sys, time
import errno, fcntl, hashlib, json, os, signal, socket, stat, struct, threading
stamps, rest = sys.argv[1], sys.argv[2:]
sys.argv = ["-"] + rest[rest.index("-") + 1:]
code = compile(sys.stdin.read(), "<stdin>", "exec")
open(stamps, "a").write("%d %r\n" % (os.getpid(), time.time()))
exec(code, {"__name__": "__main__"})
"#;

/// When the first relay for `home` began and when its process ended, by
/// the far side's wall clock, once it has ended.
fn life(lab: &FarLab, home: &str) -> Option<(SystemTime, SystemTime)> {
    let read = |name: &str| fs::read_to_string(lab.dir.join(name)).unwrap_or_default();
    let at = |seconds: &str| Some(UNIX_EPOCH + Duration::from_secs_f64(seconds.parse().ok()?));
    let ends = read("relay-ends");
    let (pid, ended) = ends.lines().find_map(|line| {
        let mut words = line.splitn(3, ' ');
        let (pid, ended, args) = (words.next()?, words.next()?, words.next()?);
        args.split(' ')
            .any(|arg| arg == home)
            .then_some((pid, ended))
    })?;
    let begins = read("relay-begins");
    let began = begins
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{pid} ")))?;
    Some((at(began)?, at(ended)?))
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_cut_off_relay_is_gone_by_its_deadline() {
    let path = ("PATH", "{lab}/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    let Some(lab) = FarLab::start_with(&[path]) else {
        return;
    };
    let file = |name: &str| lab.dir.join(name).display().to_string();
    fs::write(file("life.py"), LIFE).expect("the parent");
    fs::write(file("begins.py"), BEGINS).expect("the relay's start");
    fs::create_dir_all(lab.dir.join("bin")).expect("the stand-in's folder");
    let stand_in = lab.dir.join("bin").join("python3");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{given}'\nexec /usr/bin/python3 -I -S -B '{}' '{given}' '{}' '{}' '{}'\n",
        file("life.py"),
        file("begins.py"),
        file("relay-begins"),
        file("relay-ends"),
        given = file("relay-arguments"),
    );
    fs::write(&stand_in, script).expect("the stand-in");
    chmod(&stand_in, 0o755);
    let seconds = 5;
    let far = Far::new(seconds);
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Silent);
    let path = text(&home).to_owned();
    let (told, cut, cut_at) = std::thread::scope(|scope| {
        let exchange = scope.spawn(|| far.exchange(&lab.link, &path, Request::Identify));
        // Mid-exchange: the gateway has the line and says nothing.
        assert!(until(Duration::from_secs(10), || gate.seen().len() == 1));
        // Listed again until it shows: a busy machine's process list can
        // leave out a process it is reading (ticket 49).
        let mut carrying = Vec::new();
        assert!(until(Duration::from_secs(30), || {
            carrying = carriers(&lab, &path);
            !carrying.is_empty()
        }));
        assert_eq!(carrying.len(), 1, "the one ssh client that carries it");
        // SAFETY: a signal to the `ssh` client this test's own link started.
        unsafe { libc::kill(carrying[0], libc::SIGKILL) };
        let cut = Instant::now();
        (exchange.join().unwrap(), cut, SystemTime::now())
    });
    let came_back = cut.elapsed();
    println!("the exchange came back {came_back:?} after the kill");
    assert_eq!(told.said.unwrap_err(), Silent::CutOff);
    // At once: its client is gone, and nothing else is waited for. (Under
    // heavy load it took about 0.27 s; ticket 49.) Before, a slow return
    // was caught only by the far relay's end being looked for late.
    assert!(came_back < Duration::from_secs(2), "{came_back:?}");
    // The documented limit: the far relay outlives the client that carried
    // it (checked below from the far side's own note of its end: a busy
    // machine's process list can leave it out; ticket 49). Never two.
    assert!(lab.relays(&home) <= 1, "one far relay at most");
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
    let deadline = Duration::from_secs(seconds.into());
    // At its deadline the far relay is gone. Its process's end, as the far
    // side noted it, is within the deadline and a little of its own code
    // beginning (the bound of `a_silent_gateway_ends_at_the_deadline`), and
    // within the deadline and a second of the kill. Before, this side
    // looked for it in the far process list, every 40 ms, after the
    // exchanges above: on a busy machine those took seconds, and the look
    // came that much late (ticket 49).
    assert!(until(Duration::from_secs(60), || life(&lab, &path).is_some()));
    let (began, ended) = life(&lab, &path).unwrap();
    let lived = ended.duration_since(began).unwrap_or_default();
    let gone = ended.duration_since(cut_at).unwrap_or_default();
    println!("the far relay lived {lived:?}, and was gone {gone:?} after the kill");
    // It ran on to its own deadline: the kill did not end it.
    assert!(
        lived >= deadline - Duration::from_millis(100),
        "lived {lived:?}: it did not run on to its deadline"
    );
    assert!(
        lived < deadline + Duration::from_millis(1900),
        "lived {lived:?}"
    );
    assert!(gone <= deadline + Duration::from_secs(1), "{gone:?}");
    assert!(until(Duration::from_secs(10), || lab.relays(&home) == 0));
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
