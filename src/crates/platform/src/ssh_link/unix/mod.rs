//! The link on macOS and Linux: one `ssh` master this process owns, and
//! short-lived clients that speak to its control socket.
use super::{Closed, Cut, End, Failure, FarCommand, Forward, Lost, Ran, Refused, Settings, Target};
use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener},
    process::{Child, Command, Stdio},
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

mod args;
mod folder;
mod process;
mod sweep;
#[cfg(test)]
pub(crate) mod tests;
mod words;

use args::Ask;
pub use folder::Folder;

/// The system's own `ssh`, by its full path: nothing on `PATH` stands in.
pub const PROGRAM: &str = "/usr/bin/ssh";

pub struct Link {
    folder: Folder,
    settings: Settings,
    /// The socket's name in the folder.
    name: String,
    /// The socket's path as `ssh` reads it.
    control: String,
    state: Mutex<State>,
}

struct State {
    master: Option<Child>,
    forwards: Vec<Forward>,
    /// Why the master ended, once it has.
    lost: Option<Lost>,
    closed: Option<Closed>,
}

/// One request to the master at `control`, with the control time limit.
fn ask(settings: &Settings, control: &str, ask: Ask) -> Option<process::Finished> {
    let mut client = Command::new(&settings.program);
    client.args(args::client(control, ask));
    process::run(client, None, settings.control_limit, 16 * 1024).ok()
}

impl Link {
    /// Starts a master for `target` and waits until it is up. A host that
    /// is not trusted, a refused sign-in and a host that cannot be reached
    /// each come back as the reason, with nothing left running.
    pub fn open(folder: &Folder, target: &Target, settings: &Settings) -> Result<Self, Failure> {
        let name = folder.fresh()?;
        let control = folder.control(&name);
        let log = folder.log(&name).map_err(|_| Refused::Unavailable)?;
        let mut master = Command::new(&settings.program);
        master
            .args(args::master(settings, target, &control))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        let Ok(master) = master.spawn() else {
            folder.forget(&name);
            return Err(Refused::Unavailable.into());
        };
        let pid = master.id();
        let link = Self {
            folder: folder.clone(),
            settings: settings.clone(),
            name,
            control,
            state: Mutex::new(State {
                master: Some(master),
                forwards: Vec::new(),
                lost: None,
                closed: None,
            }),
        };
        // Dropping the link closes it: no master outlives a failed start.
        if link.folder.record(&link.name, pid).is_err() {
            return Err(Refused::Unavailable.into());
        }
        link.wait_up().map_err(Failure::Lost)?;
        Ok(link)
    }

    fn wait_up(&self) -> Result<(), Lost> {
        let until = Instant::now() + self.settings.connect_timeout * 2;
        loop {
            if let Some(lost) = self.ended() {
                return Err(lost);
            }
            if self.folder.is_socket(&self.name) && self.asks(Ask::Check) {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Lost::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn asks(&self, request: Ask) -> bool {
        ask(&self.settings, &self.control, request).is_some_and(|answer| answer.ok())
    }

    /// Why the master is gone, if it is. Its last words are read once.
    fn ended(&self) -> Option<Lost> {
        let mut state = self.state();
        if state.lost.is_none() {
            let over = match &mut state.master {
                Some(master) => !matches!(master.try_wait(), Ok(None)),
                None => true,
            };
            if over {
                state.lost = Some(if state.closed.is_some() {
                    Lost::Ended
                } else {
                    words::why_lost(&self.folder.said(&self.name))
                });
            }
        }
        state.lost
    }

    fn usable(&self) -> Result<(), Failure> {
        if self.state().closed.is_some() {
            return Err(Refused::Closed.into());
        }
        self.ended().map_or(Ok(()), |lost| Err(Failure::Lost(lost)))
    }

    /// Whether the master runs and answers. It ends by itself once the far
    /// side has been silent for the keepalive bound, so a dead connection
    /// shows here within that time.
    pub fn alive(&self) -> Result<(), Lost> {
        if let Some(lost) = self.ended() {
            return Err(lost);
        }
        if self.asks(Ask::Check) {
            self.folder.trim(&self.name);
            return Ok(());
        }
        // A master that just died may take a moment to be seen as ended.
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            if let Some(lost) = self.ended() {
                return Err(lost);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(Lost::TimedOut)
    }

    /// Runs one command on the far machine. `Ended` carries its exit
    /// status. `CutOff` means this side stopped waiting: the far program
    /// may still be running, and the caller must count it so.
    pub fn run(&self, command: &FarCommand) -> Result<Ran, Failure> {
        self.usable()?;
        let line = words::far_line(command.script, command.args)?;
        let mut client = Command::new(&self.settings.program);
        client.args(args::client(&self.control, Ask::Session(&line)));
        let done = process::run(client, command.stdin, command.limit, command.most_output)
            .map_err(|_| Refused::Unavailable)?;
        let end = match (done.cut, done.status.and_then(|status| status.code())) {
            (Some(cut), _) => End::CutOff(cut),
            (None, Some(code)) if code != 255 => End::Ended(code),
            // 255 is `ssh`'s own failure as well as a far program's status.
            (None, code) => {
                self.alive().map_err(Failure::Lost)?;
                code.map_or(End::CutOff(Cut::ChannelClosed), End::Ended)
            }
        };
        Ok(Ran {
            end,
            stdout: done.stdout,
            stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
        })
    }

    /// Opens a port on this machine's 127.0.0.1 that leads to `far`, which
    /// must be a port on the far machine's 127.0.0.1.
    pub fn forward(&self, far: SocketAddr) -> Result<Forward, Failure> {
        let far = match far {
            SocketAddr::V4(far) if *far.ip() == Ipv4Addr::LOCALHOST && far.port() != 0 => {
                far.port()
            }
            _ => return Err(Refused::FarAddress.into()),
        };
        self.usable()?;
        // A free port is found, let go, and given to the master. Another
        // program can take it in between, so a refusal tries another.
        for _ in 0..5 {
            let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .and_then(|listener| listener.local_addr())
                .map_err(|_| Refused::Unavailable)?
                .port();
            let forward = Forward { local, far };
            if self.asks(Ask::Forward(forward)) {
                self.state().forwards.push(forward);
                return Ok(forward);
            }
            self.alive().map_err(Failure::Lost)?;
        }
        Err(Refused::Unavailable.into())
    }

    pub fn cancel(&self, forward: Forward) -> Result<(), Failure> {
        self.usable()?;
        {
            let mut state = self.state();
            let at = state.forwards.iter().position(|held| *held == forward);
            state.forwards.remove(at.ok_or(Refused::NotForwarded)?);
        }
        if self.asks(Ask::Cancel(forward)) {
            return Ok(());
        }
        self.alive().map_err(Failure::Lost)?;
        Err(Refused::NotForwarded.into())
    }

    pub fn master_pid(&self) -> Option<u32> {
        self.state().master.as_ref().map(Child::id)
    }

    /// Cancels the forwards, asks the master to exit, and confirms its
    /// process gone before the socket, the record and the log are removed.
    /// A master that does not leave when asked is ended by signal.
    pub fn close(&self) -> Closed {
        let mut state = self.state();
        if let Some(closed) = state.closed {
            return closed;
        }
        let mut cancelled = 0;
        let mut reaped = true;
        let mut exit = None;
        if let Some(mut master) = state.master.take() {
            let mut status = master.try_wait().ok().flatten();
            if status.is_some() && state.lost.is_none() {
                // It ended by itself: keep why before its log is removed.
                state.lost = Some(words::why_lost(&self.folder.said(&self.name)));
            }
            if status.is_none() && self.folder.is_socket(&self.name) {
                for forward in std::mem::take(&mut state.forwards) {
                    cancelled += usize::from(self.asks(Ask::Cancel(forward)));
                }
                self.asks(Ask::Exit);
                status = process::wait(&mut master, self.settings.control_limit);
            }
            if status.is_none() {
                // SAFETY: a signal to this process's own child, not yet reaped.
                unsafe { libc::kill(master.id() as libc::pid_t, libc::SIGTERM) };
                status = process::wait(&mut master, Duration::from_secs(2));
            }
            if status.is_none() {
                let _ = master.kill();
                status = master.wait().ok();
            }
            reaped = status.is_some();
            exit = status.and_then(|status| status.code());
        }
        if reaped {
            self.folder.forget(&self.name);
        }
        let closed = Closed {
            exit,
            cancelled,
            gone: reaped && !self.folder.is_socket(&self.name),
        };
        state.lost.get_or_insert(Lost::Ended);
        state.closed = Some(closed);
        closed
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.close();
    }
}
