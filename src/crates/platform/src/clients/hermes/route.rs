//! The way to the gateways of one machine. Three things are asked of it,
//! and they are all the connection knows of where a gateway is: which
//! Hermes homes the machine has, one exchange with the control socket of
//! one of them, and a stream to a gateway the model accepted. The
//! connection names no path, no socket and no address of its own; it
//! asks the route, and hands back what the route gave it.
//!
//! The gateway of this machine is reached by `local.rs`. Another way to
//! a gateway is another `Route`, and nothing that follows a gateway
//! changes for it.
use super::control::Silent;
use agent_hud_agents::hermes::gateway::{Failure, Instance};
use std::{
    borrow::Cow,
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) mod local;

/// One Hermes home on the machine a route leads to. Only a route makes
/// one and only a route reads where it is: the connection gets it from
/// `homes`, hands it back to `exchange`, and tells the model its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Home(PathBuf);

impl Home {
    /// The home at `path` on the route's machine, links resolved there.
    fn at(path: PathBuf) -> Self {
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// The home as its gateway names it when it says which profiles it
    /// serves: its path on its own machine.
    pub(super) fn name(&self) -> Cow<'_, str> {
        self.0.to_string_lossy()
    }
}

pub(crate) trait Route: Send + Sync {
    /// The Hermes homes on the machine, the root home first, each with
    /// its links resolved. An error where they cannot be told just now,
    /// which says nothing of which homes there are.
    fn homes(&self) -> Result<Vec<Home>, Silent>;

    /// One exchange with the control socket of `home`: `line` goes in,
    /// and what the gateway answers comes back, its first line whole.
    /// Asked only where the socket is the user's own, all the way to
    /// whoever listens on it.
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent>;

    /// A stream to `instance`, which the model's `identify()` accepted
    /// from a home of this route. Nothing but that instance is dialled.
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure>;
}
