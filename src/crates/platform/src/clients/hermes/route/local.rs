//! The route to the gateway of this machine. Its home is Hermes's own
//! (`HERMES_HOME`, or `~/.hermes`). Its control socket is asked as the
//! same user, and only where the home, the pointer, the socket and its
//! listener are the user's own (`unix.rs`). Its address is dialled as the
//! gateway gave it, and only where that is this machine.
use super::{Home, Route};
use crate::clients::hermes::control::Silent;
use agent_hud_agents::hermes::{
    gateway::{Failure, Instance},
    AGENT,
};
use std::{
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
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
    /// The one home this machine's gateway is asked in: the root home.
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        let root = std::fs::canonicalize(&self.root).map_err(|_| Silent::Absent)?;
        Ok(vec![Home::at(root)])
    }

    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        host::exchange(home.path(), line, timeout)
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

/// The folder Hermes makes for the socket of `home` where the home cannot
/// hold one.
#[cfg(all(test, unix))]
pub(in crate::clients::hermes) use unix::folder_of;
