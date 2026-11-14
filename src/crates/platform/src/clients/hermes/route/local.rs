//! The route to the gateway of this machine. Its home is Hermes's own
//! (`HERMES_HOME`, or `~/.hermes`). Its control socket is asked as the
//! same user, and only where the home, the pointer, the socket and its
//! listener are the user's own (`unix.rs`). Its address is dialled as the
//! gateway gave it, and only where that is this machine.
use super::{Home, Inventory, Route, Stop};
use crate::clients::hermes::control::Silent;
use agent_hud_agents::hermes::{
    gateway::{Failure, Instance, Profile},
    AGENT,
};
use std::{
    borrow::Cow,
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as host;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as host;

const DIAL: Duration = Duration::from_secs(3);

pub(crate) struct Local {
    /// The Hermes home, as it was found: its links are resolved each time
    /// it is asked for, since a home can be made or moved while this runs.
    root: PathBuf,
}

impl Local {
    /// The route to the Hermes home on this machine: `HERMES_HOME`, or
    /// `~/.hermes`.
    pub(crate) fn new() -> Result<Self, String> {
        let user = crate::os::home_dir()?;
        let environment = std::env::vars().collect();
        let sources = AGENT.sources.on(crate::os::OS).value().copied();
        sources
            .unwrap_or_default()
            .iter()
            .flat_map(|source| crate::os::resolve(&user, &environment, &source.location))
            .next()
            .map(|root| Self { root })
            .ok_or_else(|| "Hermes has no home on this system".to_owned())
    }

    /// The route to a Hermes home that is somewhere else on this machine.
    #[cfg(test)]
    pub(crate) fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Route for Local {
    /// The root home, then the home of each named profile, by name.
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.inventory().map(|told| told.homes)
    }

    /// The root home is told wherever it is there. The named profiles'
    /// are whole only where `profiles` and everything in it could be read.
    fn inventory(&self) -> Result<Inventory, Silent> {
        let root = std::fs::canonicalize(&self.root).map_err(|_| Silent::Absent)?;
        let mut homes = vec![Home::at(self.root.clone(), root, Profile::Default)];
        let whole = named(&self.root, &mut homes);
        Ok(Inventory { homes, whole })
    }

    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        host::exchange(home.path(), line, timeout)
    }

    /// The home's links are resolved again; a home that no longer leads
    /// anywhere goes by the name it was given.
    fn named(&self, home: &Home) -> Cow<'_, str> {
        let now = std::fs::canonicalize(home.given()).unwrap_or_else(|_| home.given().to_owned());
        Cow::Owned(now.to_string_lossy().into_owned())
    }

    /// Nothing stands between this machine and its own gateway.
    fn prepare(&self, _instance: &Instance, _stop: &dyn Stop) -> Result<(), Failure> {
        Ok(())
    }

    /// No proxy: the address is this machine's, dialled as it is. An
    /// origin that names anything else is refused before any dial.
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        let invalid = || Failure::Invalid("invalid_api_origin");
        let (_, authority) = instance.origin.split_once("://").ok_or_else(invalid)?;
        let address = authority
            .to_socket_addrs()
            .ok()
            .and_then(|mut found| found.find(|address| address.ip().is_loopback()))
            .ok_or_else(invalid)?;
        TcpStream::connect_timeout(&address, DIAL).map_err(|_| Failure::Unreachable)
    }
}

/// Adds the homes of the named profiles of the Hermes root `root`: each
/// folder in its `profiles` that Hermes would name a profile by. Says
/// whether those are all of them. They are where there is no `profiles`,
/// and where it and every folder in it could be read. They are not where
/// `profiles` cannot be opened, an entry of it cannot be read, or a
/// folder in it cannot be resolved though it is there: a home that is
/// missing from such a list is not known to be gone.
fn named(root: &Path, homes: &mut Vec<Home>) -> bool {
    use std::io::ErrorKind::NotFound;
    let folders = match std::fs::read_dir(root.join("profiles")) {
        Ok(folders) => folders,
        Err(error) => return error.kind() == NotFound,
    };
    let (mut whole, first) = (true, homes.len());
    for folder in folders {
        let Ok(folder) = folder else {
            whole = false;
            continue;
        };
        let name = folder.file_name();
        let Some(profile) = name.to_str().and_then(Profile::folder) else {
            continue;
        };
        match std::fs::canonicalize(folder.path()) {
            Ok(path) if path.is_dir() => homes.push(Home::at(folder.path(), path, profile)),
            Ok(_) => {}
            // Removed since it was listed: not there.
            Err(error) if error.kind() == NotFound => {}
            Err(_) => whole = false,
        }
    }
    homes[first..].sort_by(|a, b| a.profile().cmp(b.profile()));
    whole
}

/// The folder Hermes makes for the socket of `home` where the home cannot
/// hold one.
#[cfg(all(test, unix))]
pub(in crate::clients::hermes) use unix::{folder_of, nobody};
