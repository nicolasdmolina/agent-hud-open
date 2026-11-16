//! A lab program killed with SIGKILL: its watchdog ends what the lab
//! started within five seconds, far logins that have left `sshd`'s tree
//! among them, and nothing else (ticket 49). The other tests here are the
//! PR verifiers' probes, each of which once had the reaper end a process
//! or remove a folder that was not the lab's.
use super::{
    lab::{runs, short_base, until, Lab},
    watchdog::{identity, process, Birth, Held, Registry},
};
use crate::ssh_link::{FarCommand, Link};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Given to this test program when it is started as the lab program that
/// is killed: where it writes its lab folder once its far login runs.
const AS_A_LAB: &str = "AGENT_HUD_LAB_TO_BE_KILLED";
/// The file, in the lab, where the far login writes its own id.
const FAR: &str = "far-login";

/// Not a test of its own: the lab program the tests below start and kill.
/// A lab, an `ssh` master, and a far login that runs on. Run without its
/// variable it does nothing.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_lab_program_that_is_killed() {
    let Ok(said) = std::env::var(AS_A_LAB) else {
        return;
    };
    let Some(lab) = Lab::start() else { return };
    let link = std::sync::Arc::new(
        Link::open(&lab.folder(), &lab.target(), &lab.settings()).expect("a link"),
    );
    let script = format!("echo $$ > '{}'; exec sleep 300", lab.at(FAR).display());
    let running = std::sync::Arc::clone(&link);
    thread::spawn(move || {
        let _ = running.run(&FarCommand {
            script: &script,
            args: &[],
            stdin: None,
            limit: Duration::from_secs(600),
            most_output: 1024,
        });
    });
    assert!(until(Duration::from_secs(60), || lab.at(FAR).exists()));
    // The watchdog has seen the far login while it was below `sshd`.
    thread::sleep(Duration::from_secs(1));
    // Then its link closes: its login session ends, and the far login runs
    // on below the system. Only what the watchdog saw of it before says it
    // is the lab's.
    link.close();
    let far: u32 = fs::read_to_string(lab.at(FAR))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(until(Duration::from_secs(30), || process(far)
        .is_some_and(|(_, parent, _)| parent == 1)));
    fs::write(said, lab.at("").display().to_string()).unwrap();
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

/// Not a test of its own: a lab program that kills itself the moment its
/// `ssh` master has started, before the watchdog could have seen it below
/// the program. Run without its variable it does nothing.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_lab_program_that_dies_as_its_master_starts() {
    let Ok(said) = std::env::var(AS_A_LAB) else {
        return;
    };
    let Some(lab) = Lab::start() else { return };
    fs::write(said, lab.at("").display().to_string()).unwrap();
    let link = Link::open(&lab.folder(), &lab.target(), &lab.settings());
    std::mem::forget(link);
    std::mem::forget(lab);
    // SAFETY: this program ends itself, as a time limit would end it.
    unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
}

/// A lab program that dies as its `ssh` master starts: the master and the
/// lab's `sshd` are ended all the same, being handed to the watchdog the
/// moment they started.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_lab_program_that_dies_as_it_starts_leaves_nothing() {
    let place = tempfile::tempdir().unwrap();
    let said = place.path().join("lab");
    let ended = Command::new(std::env::current_exe().unwrap())
        .args([
            "ssh_link::unix::tests::reaping::a_lab_program_that_dies_as_its_master_starts",
            "--exact",
            "--ignored",
        ])
        .env(AS_A_LAB, &said)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!ended.success(), "it killed itself");
    let dir = fs::read_to_string(&said).unwrap();
    let dir = dir.trim_end_matches('/').to_owned();
    let gone = until(Duration::from_secs(5), || {
        naming(&dir, &[]).is_empty() && !Path::new(&dir).exists()
    });
    assert!(gone, "{:?}", naming(&dir, &[]));
}

/// A lab program, running, and what it started.
struct Running {
    program: Child,
    dir: String,
    far: u32,
    _place: tempfile::TempDir,
}

fn running() -> Running {
    let place = tempfile::tempdir().unwrap();
    let said = place.path().join("lab");
    let program = Command::new(std::env::current_exe().unwrap())
        .args([
            "ssh_link::unix::tests::reaping::a_lab_program_that_is_killed",
            "--exact",
            "--ignored",
        ])
        .env(AS_A_LAB, &said)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(until(Duration::from_secs(120), || said.exists()));
    let dir = fs::read_to_string(&said).unwrap();
    let dir = dir.trim_end_matches('/').to_owned();
    let far = fs::read_to_string(format!("{dir}/{FAR}")).unwrap();
    Running {
        program,
        dir,
        far: far.trim().parse().unwrap(),
        _place: place,
    }
}

/// The processes now whose arguments name `dir`, but `except`.
fn naming(dir: &str, except: &[u32]) -> Vec<u32> {
    let listed = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains(dir))
        .filter_map(|line| line.trim_start().split_once(' ')?.0.parse().ok())
        .filter(|pid| !except.contains(pid))
        .collect()
}

/// Kills the program, and says whether within five seconds nothing of its
/// lab is left: no process naming its folder, not the far login, and not
/// the folder. `canaries` are not counted as the lab's.
fn killed(mut run: Running, canaries: &[u32]) -> (bool, Duration) {
    let lab = naming(&run.dir, canaries);
    println!(
        "  the lab: naming its folder {lab:?}, the far login {}",
        run.far
    );
    assert!(!lab.is_empty(), "the lab's sshd at least: {lab:?}");
    assert!(runs(run.far));
    // SAFETY: a signal to the program this test started.
    unsafe { libc::kill(run.program.id() as libc::pid_t, libc::SIGKILL) };
    let killed = Instant::now();
    let _ = run.program.wait();
    let gone = until(Duration::from_secs(5), || {
        naming(&run.dir, canaries).is_empty() && !runs(run.far) && !Path::new(&run.dir).exists()
    });
    (gone, killed.elapsed())
}

/// A process that is not the lab's: an idle `python3` with these
/// arguments.
fn canary(args: &[&str]) -> Child {
    Command::new("/usr/bin/python3")
        .args(["-I", "-S", "-B", "-c", "import time; time.sleep(120)"])
        .args(args)
        .stdin(Stdio::null())
        .spawn()
        .unwrap()
}

fn alive(child: &mut Child) -> bool {
    let alive = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    alive
}

/// The verifier's sibling canary: a process beside the lab that names its
/// folder, as a person looking at its log would. The far login runs below
/// the system by then: its link was closed before the program was killed.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_killed_lab_program_s_lab_ends_and_nothing_else() {
    let run = running();
    let mut beside = canary(&[&format!("{}/sshd.log", run.dir)]);
    let (gone, took) = killed(run, &[beside.id()]);
    let alive = alive(&mut beside);
    println!("  gone {gone} after {took:?}; the canary alive: {alive}");
    assert!(gone);
    assert!(
        alive,
        "the watchdog ended a process that only named the lab"
    );
}

/// The verifier's forged stale folder: a folder of the lab's name, which
/// the lab never made, with an owner file and a roots file naming an
/// unrelated process. Nothing of it is touched.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_forged_lab_folder_is_never_touched() {
    let forged = short_base().join(format!("ahl-forged-{}", std::process::id()));
    fs::create_dir_all(&forged).unwrap();
    let mut named = canary(&[]);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        forged.join("owner"),
        format!("0123456789abcdef0123456789abcdef 1 {clock}\n"),
    )
    .unwrap();
    fs::write(forged.join("roots"), format!("{} {clock}\n", named.id())).unwrap();
    fs::write(forged.join("keep"), "not the lab's").unwrap();
    let run = running();
    let (gone, _) = killed(run, &[named.id()]);
    let alive = alive(&mut named);
    let kept = forged.join("keep").exists();
    let _ = fs::remove_dir_all(&forged);
    assert!(gone);
    assert!(alive, "a process named in a forged folder was ended");
    assert!(kept, "a forged folder was removed");
}

/// The verifier's sourced startup file: an unrelated shell that reads the
/// lab's own startup file, and writes itself into the lab's folder, is not
/// the lab's.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_process_that_writes_itself_into_a_lab_is_not_the_lab_s() {
    let run = running();
    let startup = format!("{}/shell/.zshenv", run.dir);
    let mut shell = Command::new("/bin/zsh")
        .args(["-c", &format!("source '{startup}'; exec sleep 120")])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for name in ["roots", "owner", "registry"] {
        let line = format!("{} {clock}\n", shell.id());
        fs::write(format!("{}/{name}", run.dir), line).unwrap();
    }
    let (gone, _) = killed(run, &[shell.id()]);
    let alive = alive(&mut shell);
    assert!(gone);
    assert!(alive, "a process that wrote itself into the lab was ended");
}

/// The verifier's same-second replacement: a process with an id the
/// registry holds, but another birth, here a microsecond apart, is a later
/// one and is left. One whose birth is the noted one is ended.
#[test]
fn a_process_given_a_lab_process_s_id_is_left() {
    let mut other = canary(&[]);
    let mut noted = canary(&[]);
    let birth = |pid: u32| process(pid).expect("a process").2;
    let (was, is) = (birth(other.id()), birth(noted.id()));
    let mut registry = Registry::default();
    registry.add(other.id(), Birth(was.0, was.1 + 1));
    registry.add(noted.id(), is);
    registry.reap(std::process::id());
    let ended = until(Duration::from_secs(5), || {
        !runs(noted.id()) || noted.try_wait().is_ok_and(|status| status.is_some())
    });
    let left = alive(&mut other);
    let _ = noted.kill();
    let _ = noted.wait();
    assert!(ended, "the noted process was not ended");
    assert!(left, "a process of another birth was ended");
}

/// A folder made beside the test, with a file in it.
fn folder(base: &Path, name: &str, file: &str) -> PathBuf {
    let dir = base.join(name);
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join(file), file).unwrap();
    dir
}

/// The verifier's mid-cleanup swap: the lab folder moves aside and another
/// takes its path. The other is left whole; the lab's is emptied where it
/// went.
#[test]
fn a_folder_swapped_in_during_cleanup_is_left() {
    let base = tempfile::tempdir().unwrap();
    let lab = folder(base.path(), "ahl-lab", "the lab's");
    let held = Held::open(&lab, identity(&lab).unwrap()).expect("held");
    let aside = base.path().join("aside");
    fs::rename(&lab, &aside).unwrap();
    folder(base.path(), "ahl-lab", "not the lab's");
    held.remove();
    assert!(
        lab.join("not the lab's").exists(),
        "the swapped-in folder was emptied"
    );
    assert!(!aside.join("the lab's").exists());
}

/// A link is never held, nor a folder of another identity, and emptying a
/// folder removes a link in it as a link, never what it points at.
#[test]
fn a_link_is_never_followed() {
    let base = tempfile::tempdir().unwrap();
    let away = folder(base.path(), "away", "kept");
    let link = base.path().join("ahl-link");
    std::os::unix::fs::symlink(&away, &link).unwrap();
    assert!(Held::open(&link, identity(&link).unwrap()).is_none());
    assert!(Held::open(&away, (0, 0)).is_none());
    let lab = folder(base.path(), "ahl-lab", "the lab's");
    std::os::unix::fs::symlink(&away, lab.join("into")).unwrap();
    let held = Held::open(&lab, identity(&lab).unwrap()).expect("held");
    held.remove();
    assert!(away.join("kept").exists(), "a link in the lab was followed");
    assert!(!lab.exists());
}

/// The third verifier's mixed snapshot: the parent's line is from before
/// its id went to another process, the child's from after. The child of
/// that later process is not taken, and is not ended.
#[test]
fn a_child_of_a_later_process_given_a_lab_process_s_id_is_left() {
    use std::io::BufRead;
    let mut parent = Command::new("/usr/bin/python3")
        .args([
            "-I",
            "-S",
            "-B",
            "-c",
            "import subprocess, time; p = subprocess.Popen(['/bin/sleep', '60']); print(p.pid, flush=True); time.sleep(60)",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let child: u32 = line.trim().parse().unwrap();
    let (pid, ppid, birth) = process(parent.id()).unwrap();
    let child_now = process(child).unwrap();
    let earlier = Birth(birth.0.saturating_sub(60), birth.1);
    let mut registry = Registry::default();
    registry.add(pid, earlier);
    registry.grow(&[(pid, ppid, earlier), child_now], std::process::id());
    let taken = registry.holds(&child_now);
    registry.reap(std::process::id());
    let lives = process(child).map(|now| now.2) == Some(child_now.2);
    // SAFETY: signals to this test's own processes.
    unsafe { libc::kill(child as libc::pid_t, libc::SIGKILL) };
    let _ = parent.kill();
    let _ = parent.wait();
    assert!(!taken, "a child of another process was taken");
    assert!(lives, "a child of another process was ended");
}

/// The owner is never the system's first process, nor one that is gone or
/// of another birth.
#[test]
fn the_owner_is_never_the_system_or_a_process_that_is_gone() {
    use super::watchdog::owner;
    assert!(owner("owner 1 0 0").is_none());
    if let Some((_, _, first)) = process(1) {
        assert!(owner(&format!("owner 1 {} {}", first.0, first.1)).is_none());
    }
    let mut ended = Command::new("/bin/sleep").arg("0.1").spawn().unwrap();
    let (pid, _, born) = process(ended.id()).unwrap();
    assert_eq!(
        owner(&format!("owner {pid} {} {}", born.0, born.1)),
        Some((pid, born))
    );
    assert_eq!(
        owner(&format!("owner {pid} {} {}", born.0, born.1 + 1)),
        None
    );
    ended.wait().unwrap();
    assert_eq!(owner(&format!("owner {pid} {} {}", born.0, born.1)), None);
}

/// The third verifier's startup probe: the owner is gone before the
/// watchdog begins. What the owner had sent is ended and removed all the
/// same, and nothing else.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_watchdog_whose_owner_is_gone_before_it_begins_ends_what_it_was_given() {
    let base = tempfile::tempdir().unwrap();
    let dir = folder(base.path(), "ahl-owner-gone", "the lab's");
    let mut owner = Command::new("/bin/sleep").arg("0.3").spawn().unwrap();
    let (pid, _, born) = process(owner.id()).unwrap();
    let mut given = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let mut beside = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let (given_pid, _, given_born) = process(given.id()).unwrap();
    owner.wait().unwrap();
    let mut watchdog = super::reaper::begin((pid, born));
    let (dev, ino) = identity(&dir).unwrap();
    let lines = format!(
        "pid {given_pid} {} {}\ndir {dev} {ino} {}\n",
        given_born.0,
        given_born.1,
        dir.display()
    );
    std::io::Write::write_all(&mut watchdog.pipe, lines.as_bytes()).unwrap();
    drop(watchdog);
    let done = until(Duration::from_secs(5), || {
        given.try_wait().is_ok_and(|status| status.is_some()) && !dir.exists()
    });
    let alive = beside.try_wait().unwrap().is_none();
    let _ = given.kill();
    let _ = given.wait();
    let _ = beside.kill();
    let _ = beside.wait();
    assert!(done, "what a gone owner had sent was left");
    assert!(alive, "a process that was not sent was ended");
}

/// Given to this test program as a nested lab program, outer or inner:
/// the folder they work in.
const AS_AN_OUTER: &str = "AGENT_HUD_LAB_NESTED_OUTER";
const AS_AN_INNER: &str = "AGENT_HUD_LAB_NESTED_INNER";

/// Not a test of its own: the two lab programs of the nested test, each
/// with a lab folder and a watchdog of its own, the inner one started by
/// the outer. Run without their variables it does nothing.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_nested_lab_program() {
    let me = "ssh_link::unix::tests::reaping::a_nested_lab_program";
    // The inner program, held as long as the outer runs.
    let mut _inner = None;
    if let Ok(base) = std::env::var(AS_AN_OUTER) {
        let dir = folder(Path::new(&base), "outer", "the outer lab's");
        super::reaper::watch(&dir);
        let inner = Command::new(std::env::current_exe().unwrap())
            .args([me, "--exact", "--ignored"])
            .env_remove(AS_AN_OUTER)
            .env(AS_AN_INNER, &base)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        fs::write(Path::new(&base).join("inner-pid"), inner.id().to_string()).unwrap();
        _inner = Some(inner);
    } else if let Ok(base) = std::env::var(AS_AN_INNER) {
        let dir = folder(Path::new(&base), "inner", "the inner lab's");
        super::reaper::watch(&dir);
        fs::write(Path::new(&base).join("ready"), "").unwrap();
    } else {
        return;
    }
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

/// The third verifier's nested watchdogs: the outer lab program is killed
/// after its watchdog has seen the inner program and its watchdog below
/// it. The outer one ends the inner program, but not its watchdog, which
/// then removes the inner folder; the outer removes its own.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_nested_lab_program_s_folder_is_removed_too() {
    let base = tempfile::tempdir().unwrap();
    let mut outer = Command::new(std::env::current_exe().unwrap())
        .args([
            "ssh_link::unix::tests::reaping::a_nested_lab_program",
            "--exact",
            "--ignored",
        ])
        .env(AS_AN_OUTER, base.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(until(Duration::from_secs(60), || base
        .path()
        .join("ready")
        .exists()));
    let inner: u32 = fs::read_to_string(base.path().join("inner-pid"))
        .unwrap()
        .parse()
        .unwrap();
    // The outer watchdog looks every ~200 ms: it has seen the inner tree.
    thread::sleep(Duration::from_millis(1500));
    // SAFETY: a signal to the program this test started.
    unsafe { libc::kill(outer.id() as libc::pid_t, libc::SIGKILL) };
    let _ = outer.wait();
    let (outer_dir, inner_dir) = (base.path().join("outer"), base.path().join("inner"));
    let clean = until(Duration::from_secs(6), || {
        !outer_dir.exists() && !inner_dir.exists() && !runs(inner)
    });
    println!(
        "  outer folder left: {}, inner folder left: {}, inner program running: {}",
        outer_dir.exists(),
        inner_dir.exists(),
        runs(inner)
    );
    assert!(clean);
}
