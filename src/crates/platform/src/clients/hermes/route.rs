//! The way to the gateways of one machine. Three things are asked of it,
//! and they are all the connection knows of where a gateway is: which
//! Hermes homes the machine has, one exchange with the control socket of
//! one of them, and a stream to a gateway the model accepted. Each of the
//! last two has a step that goes with it: how the home is named once its
//! gateway has answered (`named`), and what has to be in place for an
//! accepted gateway before a ticket is asked of it (`prepare`), which
//! stops when the connection does (`Stop`). The
//! connection names no path, no socket and no address of its own; it
//! asks the route, and hands back what the route gave it.
//!
//! The gateway of this machine is reached by `local.rs`. Another way to
//! a gateway is another `Route`, and nothing that follows a gateway
//! changes for it.
use super::control::Silent;
use agent_hud_agents::hermes::gateway::{Failure, Instance, Profile};
use std::{
    borrow::Cow,
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) mod local;

/// One Hermes home on the machine a route leads to. Only a route makes
/// one and only a route reads where it is: the connection gets it from
/// `homes` and hands it back to `exchange` and to `named`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Home {
    /// The home as the route was told of it, which may lead through links.
    given: PathBuf,
    /// Where that led when the homes were asked for, links resolved: the
    /// place its control socket is asked in.
    path: PathBuf,
    /// The profile whose home it is, by its place on the machine: the
    /// root is the default profile's, `profiles/<name>` a named one's.
    profile: Profile,
}

impl Home {
    /// The home of `profile`, which the route knows as `given` and which
    /// is at `path` on the route's machine, links resolved there.
    fn at(given: PathBuf, path: PathBuf, profile: Profile) -> Self {
        Self {
            given,
            path,
            profile,
        }
    }

    /// The profile whose home this is. It names no place.
    pub(crate) fn profile(&self) -> &Profile {
        &self.profile
    }

    fn given(&self) -> &Path {
        &self.given
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether whoever asked a route for something still wants it. What a
/// route does for `prepare` can take time, and the one who asked may stop
/// meanwhile (client hooks turned off): the route looks here while it
/// waits, and gives up as soon as this says stopped.
pub(crate) trait Stop: Sync {
    /// Waits `time`, or less where the asker stops first, and says whether
    /// it has stopped. No time at all only asks. Nothing here calls it
    /// yet: the route to this machine has nothing to prepare, so nothing
    /// to wait for.
    #[allow(dead_code)]
    fn wait(&self, time: Duration) -> bool;
}

pub(crate) trait Route: Send + Sync {
    /// The Hermes homes on the machine, each with its links resolved: the
    /// root home, which is the default profile's, and the home of every
    /// named profile. An error where they cannot be told just now, which
    /// says nothing of which homes there are; and a home that is not
    /// listed is not known to be gone.
    fn homes(&self) -> Result<Vec<Home>, Silent>;

    /// One exchange with the control socket of `home`: `line` goes in,
    /// and what the gateway answers comes back, its first line whole.
    /// Asked only where the socket is the user's own, all the way to
    /// whoever listens on it.
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent>;

    /// The name `home` goes by now, as a gateway writes the homes it
    /// serves: its path on its own machine, links resolved once more.
    /// Where a home leads can change while its gateway is asked, and what
    /// the gateway answered is held against the home as it is after the
    /// answer came, not as it was when the question went.
    fn named(&self, home: &Home) -> Cow<'_, str>;

    /// Puts in place what a stream to `instance` needs, before a ticket is
    /// asked for: a ticket lasts thirty seconds, and nothing slow may come
    /// between it and the dial. `instance` is one the model's `identify()`
    /// accepted from a home of this route. Asked before every ticket, so a
    /// route keeps what it made and makes it once. Whatever in it waits,
    /// waits on `stop` too: once the asker has stopped, the route ends what
    /// it began for this call and says so with a failure, and nothing of
    /// it goes on afterwards.
    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure>;

    /// A stream to `instance`, which the model's `identify()` accepted
    /// from a home of this route and `prepare` was asked for. Nothing but
    /// that instance is dialled.
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure>;
}
