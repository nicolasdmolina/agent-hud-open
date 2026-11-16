//! Labs end with the test program that started them, however it ends
//! (ticket 49), and nothing else ends with them.
//!
//! A lab's processes are not all this program's children: its `sshd` runs
//! every login in a session of its own, and a far process a test cut off
//! outlives the client that carried it. A lab's `Drop` ends them, but a
//! program killed by a time limit (SIGKILL: nothing of it runs again) used
//! to leave them, and the lab folders.
//!
//! So the first lab of a program starts a watchdog (`watchdog.rs`): this
//! same test program, run as `watchdog` below, in a process group of its
//! own, which a signal to the program's group does not reach. What it does
//! and to what, it keeps in its own memory, and learns only from the
//! kernel and from a pipe only this program holds:
//!
//! - its registry starts with this program, by its id and birth (when the
//!   kernel says it began, to the microsecond), which this program finds
//!   before it starts the watchdog and sends first; the watchdog answers
//!   `ready`, and the lab does not go on before it has. Every ~200 ms the
//!   watchdog adds each process below one of the registry's, with its
//!   birth, the parent looked at again as it is then: `sshd`, the agent,
//!   `ssh` clients and masters, and each far login, also after it leaves
//!   `sshd`'s tree;
//! - each process this program starts for a lab (`sshd`, the agent, `ssh`
//!   clients and masters) comes over the pipe at once, with its birth, so
//!   one started just before the program dies is not missed;
//! - each lab folder comes over the pipe with its device and inode, as
//!   this program found them when it made the folder; the watchdog holds
//!   it open where it is that folder and not a link, and answers `held`;
//!   the lab does not go on before it has. A watchdog that cannot start,
//!   or be told, or does not answer, fails the lab.
//!
//! When this program ends, by whatever death, the pipe closes (and the
//! watchdog also sees the program's birth go). Then it stops and kills
//! each process of the registry, each one only while its birth is still
//! the one noted (but another lab program's watchdog, which ends that
//! program's lab itself), and empties each folder through its open
//! descriptor, removing the path only where it is still that folder. A process that
//! only names a lab folder, or writes into one, or whose id a lab process
//! had, is not touched. Folders of other runs are never looked at.
use super::watchdog;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Mutex, OnceLock},
    time::Duration,
};

/// What every lab folder's name begins with.
pub(crate) const PREFIX: &str = "ahl";
/// Given to this test program when it is started as a watchdog.
const AS_A_WATCHDOG: &str = "AGENT_HUD_LAB_WATCHDOG";
/// The test that is the watchdog.
const WATCHDOG: &str = "ssh_link::unix::tests::reaper::watchdog";
/// How long the watchdog has to answer.
const ANSWER: Duration = Duration::from_secs(60);

/// This program's watchdog: the pipe to it, and what it answers.
pub(crate) struct Watchdog {
    pub(crate) pipe: ChildStdin,
    pub(crate) said: mpsc::Receiver<String>,
    /// It outlives this program: it is never waited for here.
    _process: Child,
}

static WATCHDOG_OF_THIS: OnceLock<Mutex<Watchdog>> = OnceLock::new();

/// Not a test of its own: the watchdog, when this test program is started
/// as one. Run without its variable it does nothing.
#[test]
fn watchdog() {
    if std::env::var_os(AS_A_WATCHDOG).is_some() {
        watchdog::run(std::io::stdin(), std::io::stdout());
    }
}

fn written(watchdog: &mut Watchdog, line: &str) {
    let wrote = writeln!(watchdog.pipe, "{line}").and_then(|()| watchdog.pipe.flush());
    assert!(
        wrote.is_ok(),
        "the lab's watchdog cannot be told {line}: {wrote:?}"
    );
}

fn answer(watchdog: &Watchdog, what: &str) -> String {
    let said = watchdog.said.recv_timeout(ANSWER);
    said.unwrap_or_else(|why| panic!("the lab's watchdog did not answer {what}: {why}"))
}

/// Hands the lab folder `dir`, which this program made, to its watchdog,
/// which is started with the first, and waits until the watchdog holds it.
pub(crate) fn watch(dir: &Path) {
    let (dev, ino) = watchdog::identity(dir)
        .unwrap_or_else(|| panic!("the lab folder {} is not there", dir.display()));
    let mut watchdog = WATCHDOG_OF_THIS
        .get_or_init(|| Mutex::new(start()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    written(&mut watchdog, &format!("dir {dev} {ino} {}", dir.display()));
    let said = answer(&watchdog, "a lab folder");
    assert_eq!(
        said,
        "held",
        "the lab's watchdog did not take {}",
        dir.display()
    );
}

/// Hands a process this program has just started to its watchdog, by its
/// id and birth, where a lab has started in this program: the lab's `sshd`
/// and agent, and every `ssh` client and master (`opening.rs`,
/// `process.rs`). The watchdog's own look every ~200 ms finds the rest.
pub(crate) fn spawned(pid: u32) {
    let Some(watchdog) = WATCHDOG_OF_THIS.get() else {
        return;
    };
    let Some((_, _, watchdog::Birth(seconds, micros))) = watchdog::process(pid) else {
        return;
    };
    let mut watchdog = watchdog
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    written(&mut watchdog, &format!("pid {pid} {seconds} {micros}"));
}

/// Starts a watchdog for the program `owner`, told first who that is.
pub(crate) fn begin(owner: (u32, watchdog::Birth)) -> Watchdog {
    let mut child = Command::new(std::env::current_exe().expect("this test program"))
        .args([WATCHDOG, "--exact", "--quiet"])
        .env(AS_A_WATCHDOG, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap_or_else(|why| panic!("the lab's watchdog did not start: {why}"));
    let pipe = child.stdin.take().expect("the watchdog's pipe");
    let out = child.stdout.take().expect("what the watchdog says");
    let (tell, said) = mpsc::channel();
    // libtest's own lines are not the watchdog's answers.
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if matches!(line.as_str(), "ready" | "held" | "refused") && tell.send(line).is_err() {
                return;
            }
        }
    });
    let mut watchdog = Watchdog {
        pipe,
        said,
        _process: child,
    };
    let (pid, watchdog::Birth(seconds, micros)) = owner;
    written(&mut watchdog, &format!("owner {pid} {seconds} {micros}"));
    watchdog
}

fn start() -> Watchdog {
    // Who this program is, as the kernel has it, before the watchdog runs:
    // it is never left to find that out for itself.
    let me = std::process::id();
    let (_, _, born) = watchdog::process(me).expect("this program, as the kernel has it");
    let watchdog = begin((me, born));
    assert_eq!(answer(&watchdog, "that it began"), "ready");
    // A program that loses the one that started it ends: no lab test of
    // it is waited for any more, and its watchdog then does its work.
    // SAFETY: a plain query of this process.
    let parent = unsafe { libc::getppid() };
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(100));
        // SAFETY: a plain query; `_exit` ends this process at once.
        if unsafe { libc::getppid() } != parent {
            unsafe { libc::_exit(101) };
        }
    });
    watchdog
}
