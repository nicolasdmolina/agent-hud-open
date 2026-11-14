//! One short process, waited for against a deadline, and the checks on a
//! master's process.
//!
//! The wait never blocks on a pipe. A session's pipes are handed to the
//! master, which can hold them after the client is gone, so they are read
//! without blocking and the wait ends at the deadline whoever holds what.
use super::super::Cut;
use std::{
    io::{self, ErrorKind, Read, Write},
    os::fd::AsRawFd,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

/// The most standard error kept. The rest is read and dropped, so the far
/// program is not held up on it.
const MOST_ERROR: usize = 8 * 1024;
const TICK_MS: i32 = 50;
/// How long output is still read after the process ended.
const AFTER_EXIT: Duration = Duration::from_millis(250);

pub(super) struct Finished {
    /// None where the wait was cut.
    pub status: Option<ExitStatus>,
    pub cut: Option<Cut>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Finished {
    pub(super) fn ok(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }
}

/// Ends and reaps the process however the wait is left.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(
    mut command: Command,
    input: Option<&[u8]>,
    limit: Duration,
    most_output: usize,
) -> io::Result<Finished> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Reaped(command.spawn()?);
    let until = Instant::now() + limit;
    let mut stdin = child.0.stdin.take();
    let mut stdout = child.0.stdout.take();
    let mut stderr = child.0.stderr.take();
    let descriptors = [
        stdin.as_ref().map(AsRawFd::as_raw_fd),
        stdout.as_ref().map(AsRawFd::as_raw_fd),
        stderr.as_ref().map(AsRawFd::as_raw_fd),
    ];
    for descriptor in descriptors.into_iter().flatten() {
        nonblocking(descriptor)?;
    }
    let mut input = input.unwrap_or_default();
    let mut done = Finished {
        status: None,
        cut: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let mut exited: Option<Instant> = None;
    loop {
        if input.is_empty() {
            // Closed, so the far program reads the end of its input.
            stdin = None;
        }
        let wanted = |descriptor: Option<i32>, events| libc::pollfd {
            fd: descriptor.unwrap_or(-1),
            events,
            revents: 0,
        };
        let mut waits = [
            wanted(stdin.as_ref().map(AsRawFd::as_raw_fd), libc::POLLOUT),
            wanted(stdout.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
            wanted(stderr.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
        ];
        // SAFETY: three descriptors this function owns; a negative one is skipped.
        unsafe { libc::poll(waits.as_mut_ptr(), 3, TICK_MS) };
        if let Some(pipe) = &mut stdin {
            match pipe.write(input) {
                Ok(count) => input = &input[count..],
                Err(error) if retry(&error) => {}
                Err(_) => input = &[],
            }
        }
        drain(&mut stdout, &mut done.stdout, most_output.saturating_add(1));
        drain(&mut stderr, &mut done.stderr, MOST_ERROR);
        if done.stdout.len() > most_output {
            done.stdout.truncate(most_output);
            done.cut = Some(Cut::TooMuchOutput);
            break;
        }
        if exited.is_none() {
            done.status = child.0.try_wait()?;
            exited = done.status.map(|_| Instant::now());
        }
        let drained = stdout.is_none() && stderr.is_none();
        if exited.is_some_and(|at| drained || at.elapsed() > AFTER_EXIT) {
            break;
        }
        if Instant::now() >= until {
            done.cut = Some(Cut::TimeLimit);
            break;
        }
    }
    if done.cut.is_some() {
        done.status = None;
    }
    Ok(done)
}

fn retry(error: &io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

/// Reads what is there now, keeping at most `most` bytes. The pipe is let
/// go at its end.
fn drain(pipe: &mut Option<impl Read>, into: &mut Vec<u8>, most: usize) {
    let mut chunk = [0u8; 16 * 1024];
    while let Some(open) = pipe {
        match open.read(&mut chunk) {
            Ok(0) => *pipe = None,
            Ok(count) => {
                let room = most.saturating_sub(into.len()).min(count);
                into.extend_from_slice(&chunk[..room]);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(_) => *pipe = None,
        }
    }
}

fn nonblocking(descriptor: i32) -> io::Result<()> {
    // SAFETY: a descriptor this process holds open; only its flags change.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Waits for a child to end, for at most `limit`.
pub(super) fn wait(master: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let until = Instant::now() + limit;
    loop {
        if let Ok(Some(status)) = master.try_wait() {
            return Some(status);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether `pid` is a master of this folder: a running process whose
/// arguments name `control`. A reused pid is some other program and is not
/// one; nor is a master that ended and waits to be reaped.
pub(super) fn master_runs(pid: u32, control: &str) -> bool {
    let mut ps = Command::new("/bin/ps");
    ps.args(["-ww", "-o", "args=", "-p", &pid.to_string()]);
    match run(ps, None, Duration::from_secs(3), 64 * 1024) {
        Ok(listed) if listed.status.is_some() => {
            String::from_utf8_lossy(&listed.stdout).contains(&format!("ControlPath={control}"))
        }
        // The listing could not be read: say it runs, so nothing of its is
        // removed.
        _ => true,
    }
}

/// Every process of this user whose arguments name a control path that
/// starts with `start`: its pid, and the socket's name in the folder.
/// None where the list could not be read whole: then nothing is known of
/// who runs, and the caller confirms nothing from it.
pub(super) fn naming(start: &str) -> Option<Vec<(u32, String)>> {
    let mut ps = Command::new("/bin/ps");
    ps.args(["-xww", "-o", "pid=,args="]);
    let listed = run(ps, None, Duration::from_secs(5), 4 * 1024 * 1024).ok()?;
    if !listed.ok() {
        return None;
    }
    let start = format!("ControlPath={start}");
    let named = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, args) = line.trim_start().split_once(' ')?;
            let name = args.split_once(&start)?.1.split('"').next()?;
            Some((pid.parse().ok()?, name.to_owned()))
        })
        .collect();
    Some(named)
}

/// Whether the master on `pid` is gone within `wait`.
pub(super) fn gone_within(pid: u32, control: &str, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    loop {
        if !master_runs(pid, control) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Ends a master that did not leave when asked: asked again by signal,
/// then killed. Each signal goes only to a process just seen to be it.
pub(super) fn end(pid: u32, control: &str) -> bool {
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        if pid <= 1 || !master_runs(pid, control) {
            return true;
        }
        // SAFETY: a signal to one process, seen above to be this folder's master.
        unsafe { libc::kill(pid as libc::pid_t, signal) };
        if gone_within(pid, control, Duration::from_secs(2)) {
            return true;
        }
    }
    false
}
