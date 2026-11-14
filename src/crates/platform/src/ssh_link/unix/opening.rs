//! Opening a link: the master is started and waited for until it answers
//! on its control socket. The wait can be given up from outside, since a
//! sign-in through a slow proxy can take most of a minute and whoever
//! asked for the link may want it no more by then.
use super::{args, args::Ask, Failure, Folder, Link, Lost, Refused, Settings, State, Target};
use std::{
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

impl Link {
    /// Starts a master for `target` and waits until it is up. A host that
    /// is not trusted, a refused sign-in and a host that cannot be reached
    /// each come back as the reason, with nothing left running.
    pub fn open(folder: &Folder, target: &Target, settings: &Settings) -> Result<Self, Failure> {
        Self::open_while(folder, target, settings, &|| true)
    }

    /// [`Link::open`], given up as soon as `wanted` says no: the master is
    /// ended, nothing is left running, and the link is refused as closed.
    pub fn open_while(
        folder: &Folder,
        target: &Target,
        settings: &Settings,
        wanted: &dyn Fn() -> bool,
    ) -> Result<Self, Failure> {
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
        link.wait_up(wanted)?;
        Ok(link)
    }

    fn wait_up(&self, wanted: &dyn Fn() -> bool) -> Result<(), Failure> {
        let until = Instant::now() + self.settings.connect_timeout * 2;
        loop {
            if !wanted() {
                return Err(Refused::Closed.into());
            }
            if let Some(lost) = self.ended() {
                return Err(Failure::Lost(lost));
            }
            if self.folder.is_socket(&self.name) && self.asks(Ask::Check) {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Failure::Lost(Lost::TimedOut));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Whether the master's process still runs, without asking it
    /// anything: a master that was killed, or ended itself after its
    /// keepalives went unanswered, shows here at once and for nothing.
    /// [`Link::alive`] also asks it, which starts a process.
    pub fn running(&self) -> Result<(), Lost> {
        self.ended().map_or(Ok(()), Err)
    }
}
