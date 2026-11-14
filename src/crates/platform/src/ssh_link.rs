//! One owned SSH connection to a far machine: open it, run a command over
//! it, forward a local port to a far loopback port, cancel a forward, ask
//! whether it is alive, close it. It names no client and knows nothing of
//! what runs at the far end.
//!
//! The system's `ssh` does the work, so the user's host aliases and proxy
//! commands hold. Every setting that could weaken the connection is given
//! on the command line, where no configuration file can change it
//! (`unix/args.rs`). A host whose key is unknown or has changed is refused,
//! and `known_hosts` is never written. The agent may sign in and is never
//! forwarded.
//!
//! The master is a child this process owns, in one private folder with
//! every other master it ever started ([`Folder`]). Its output goes to a
//! file there, never to a pipe. Commands and forwards go through its
//! control socket and can reach nothing else.
//!
//! **A time limit ends the local wait and nothing more.** [`End::CutOff`]
//! says the far program may still be running: closing the channel does not
//! stop a program that ignores it. A caller runs only far programs that
//! carry their own deadline, and counts a cut-off one as running until
//! that deadline has passed.
use std::{path::PathBuf, time::Duration};

mod target;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(unix)]
use unix as host;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as host;

pub use host::{Folder, Link};
pub use target::Target;

/// The longest control socket path: a socket address holds 104 bytes with
/// its end, and `ssh` adds 17 characters to the path while it binds.
pub const LONGEST_SOCKET_PATH: usize = 104 - 1 - 17;

/// How `ssh` is run. [`Settings::system`] is what the app uses; a lab names
/// its own configuration file and shorter waits.
#[derive(Clone, Debug)]
pub struct Settings {
    pub program: PathBuf,
    /// The configuration file the master reads. None: the user's own.
    pub config: Option<PathBuf>,
    pub connect_timeout: Duration,
    /// A keepalive goes out this often, and the master ends after
    /// `alive_count` of them go unanswered.
    pub alive_interval: Duration,
    pub alive_count: u32,
    /// How long one request to the master's control socket may take.
    pub control_limit: Duration,
}

impl Settings {
    pub fn system() -> Self {
        Self {
            program: PathBuf::from(host::PROGRAM),
            config: None,
            connect_timeout: Duration::from_secs(30),
            alive_interval: Duration::from_secs(15),
            alive_count: 3,
            control_limit: Duration::from_secs(5),
        }
    }
    /// A link whose far side went silent is lost within this long.
    pub fn keepalive_bound(&self) -> Duration {
        self.alive_interval * self.alive_count
    }
}

/// Why a link is not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    /// The host's key is not in `known_hosts`.
    NotTrusted,
    /// The host's key is not the one in `known_hosts`.
    KeyChanged,
    SignInRefused,
    CannotReach,
    TimedOut,
    /// The master is gone and said nothing of why (killed, or the Mac slept).
    Ended,
}

impl Lost {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotTrusted => "not trusted yet",
            Self::KeyChanged => "host key changed",
            Self::SignInRefused => "sign-in refused",
            Self::CannotReach => "cannot reach",
            Self::TimedOut => "timed out",
            Self::Ended => "connection ended",
        }
    }
}

/// What the link would not do. `ssh` was not run for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Address,
    KeyPath,
    /// The control folder is a link or not a folder.
    FolderKind,
    FolderOwner,
    FolderMode,
    /// The folder's path holds a character `ssh` would read as its own.
    FolderPath,
    PathTooLong,
    /// A forward leads to 127.0.0.1 on the far side and nowhere else.
    FarAddress,
    NotForwarded,
    CommandTooLong,
    HereDocument,
    /// `ssh` could not be started, or the folder could not be written.
    Unavailable,
    Closed,
    NotImplemented,
}

impl Refused {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Address => "unusable address",
            Self::KeyPath => "unusable key path",
            Self::FolderKind => "control folder is not a folder",
            Self::FolderOwner => "control folder belongs to someone else",
            Self::FolderMode => "control folder is open to others",
            Self::FolderPath => "control folder path holds a character ssh reads",
            Self::PathTooLong => "control folder path too long for a socket",
            Self::FarAddress => "a forward leads only to 127.0.0.1",
            Self::NotForwarded => "no such forward",
            Self::CommandTooLong => "command line too long",
            Self::HereDocument => "here-documents are not sent",
            Self::Unavailable => "ssh could not be started",
            Self::Closed => "link is closed",
            Self::NotImplemented => "not implemented",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Refused(Refused),
    Lost(Lost),
}

impl From<Refused> for Failure {
    fn from(refused: Refused) -> Self {
        Self::Refused(refused)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Refused(refused) => refused.reason(),
            Self::Lost(lost) => lost.reason(),
        })
    }
}

/// One command for the far machine. `script` runs under `sh -c` with `args`
/// as its `$1`, `$2`…; each is quoted by [`quote`]. The line stays short: a
/// long program goes in `stdin`.
#[derive(Clone, Copy, Debug)]
pub struct FarCommand<'a> {
    pub script: &'a str,
    pub args: &'a [&'a str],
    pub stdin: Option<&'a [u8]>,
    /// How long this side waits. See [`End::CutOff`].
    pub limit: Duration,
    /// The most standard output kept; more than this cuts the command off.
    pub most_output: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub end: End,
    pub stdout: Vec<u8>,
    /// What the far program and `ssh` wrote to standard error, bounded.
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The far program ended with this status.
    Ended(i32),
    /// This side stopped waiting and closed the channel. The far program
    /// may still be running.
    CutOff(Cut),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    TimeLimit,
    TooMuchOutput,
    /// The channel closed with no exit status.
    ChannelClosed,
}

/// A local port on 127.0.0.1 that leads to a port on the far 127.0.0.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forward {
    pub local: u16,
    pub far: u16,
}

/// How a link closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    /// The master's exit code, where it ended by itself.
    pub exit: Option<i32>,
    pub cancelled: usize,
    /// The master's process is gone and its socket is not in the folder.
    pub gone: bool,
}

/// What start-up cleanup did in the control folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Masters that were running and were closed.
    pub closed: usize,
    /// Sockets removed, each after its master was confirmed gone.
    pub removed: usize,
    /// Entries left alone: a link, another user's file, or a master that
    /// would not end.
    pub left: usize,
}

/// The one function that quotes text for the far shell: a far path with a
/// quote, a space or a `$` arrives as plain text. A backslash is closed out
/// of the quotes too, since fish reads one inside them.
pub fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for character in text.chars() {
        match character {
            '\'' => quoted.push_str("'\\''"),
            '\\' => quoted.push_str("'\\\\'"),
            other => quoted.push(other),
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests;
// The Windows file names nothing of Windows, so its promise is checked on
// the hosts that can run the tests.
#[cfg(all(test, unix))]
#[allow(dead_code)]
#[path = "ssh_link/windows.rs"]
mod windows_stub;
