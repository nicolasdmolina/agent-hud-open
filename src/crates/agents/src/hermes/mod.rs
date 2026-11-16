//! Hermes.
pub mod account;
pub mod connections;
pub mod far_history;
pub mod gateway;
mod hooks;
pub mod mirror;
mod session_counts;
mod settings;
pub mod transcript;

use crate::Agent;
use crate::Discovery;
use crate::{Base, Location, Source};
use crate::{Carries, Channel, PerOs, Support};

/// Hermes's local gateway, which its desktop app talks to: the questions
/// and approvals a session waits on, and the answers to them. Agent HUD
/// asks the gateway's control socket, as the same user, for a ticket that
/// opens one WebSocket (`gateway`). A gateway without this connection is
/// followed by the hooks alone.
pub static GATEWAY: Channel = Channel {
    name: "Hermes's local gateway",
    carries: &[Carries::Requests],
    transcript_shows_pending: false,
    support: PerOs {
        macos: Support::Verified(()),
        windows: Support::Unsupported(
            "Hermes's gateway is asked through a named pipe on Windows, which is not implemented",
        ),
        linux: Support::Unverified(()),
    },
};

/// The way to the gateways of a far machine, over SSH: the system's `ssh`
/// as a master Agent HUD owns, a relay that asks each far control socket
/// as the login user, and a forwarded port for each gateway's WebSocket
/// (`platform/src/clients/hermes/route/ssh.rs`). Verified on macOS
/// against an `sshd` on loopback, with this Mac as the far side too, and
/// read-only against a far Mac; the same code never ran on Linux, and
/// Windows' OpenSSH has no control sockets to carry it.
pub static FAR_ROUTE: PerOs<()> = PerOs {
    macos: Support::Verified(()),
    windows: Support::Unsupported(
        "a far Hermes is reached over an SSH master, which Windows' OpenSSH does not have",
    ),
    linux: Support::Unverified(()),
};

/// This machine's Hermes home.
const HOME: Source = Source {
    reads_as: "hermes",
    location: Location::home(&["HERMES_HOME"], &[".hermes"], ""),
};
/// The mirrors of far machines (`mirror.rs`), in Agent HUD's own data
/// folder: `FOLDER`, named after the app's identifier, or the one
/// `AGENTHUD_DATA_DIR` names, as Cursor's turn log is declared
/// (`cursor/mod.rs`). Each is read as any Hermes `state.db` is, and its
/// own `hud_mirror` table says whose sessions it holds. Only where a far
/// Hermes can be reached (`FAR_ROUTE`).
const fn mirrors(base: Base) -> Source {
    Source {
        reads_as: "hermes",
        location: Location {
            base,
            env: &["AGENTHUD_DATA_DIR"],
            folders: &[FOLDER],
            profiles: None,
            path: mirror::ROOT,
            not_if: None,
        },
    }
}
/// The fork's data folder on every system (FORK.md).
const FOLDER: &str = "com.nickymolina.agenthud";
static MAC_SOURCES: [Source; 2] = [HOME, mirrors(Base::AppData)];
static WINDOWS_SOURCES: [Source; 1] = [HOME];
static LINUX_SOURCES: [Source; 2] = [HOME, mirrors(Base::DataHome)];
static DISCOVERY: Discovery = Discovery {
    storage: &[
        Location::home(&["HERMES_HOME"], &[".hermes"], "state.db"),
        Location::home(&["HERMES_HOME"], &[".hermes"], "profiles"),
    ],
    settings: &[
        Location::home(&["HERMES_HOME"], &[".hermes"], "config.yaml"),
        Location::home(&["HERMES_HOME"], &[".hermes"], "config.json"),
    ],
    sessions: &[
        Location::home(&["HERMES_HOME"], &[".hermes"], "state.db"),
        Location::home(&["HERMES_HOME"], &[".hermes"], "profiles"),
    ],
    commands: &["hermes"],
    ..Discovery::NONE
};

pub static AGENT: Agent = Agent {
    id: "hermes",
    name: "Hermes",
    sessions: true,
    hooks: Some(&hooks::HOOKS),
    observer: None,
    channel: Some(&GATEWAY),
    program: None,
    replies: None,
    turns: None,
    items: None,
    sources: PerOs {
        macos: Support::Verified(&MAC_SOURCES),
        windows: Support::Unverified(&WINDOWS_SOURCES),
        linux: Support::Unverified(&LINUX_SOURCES),
    },
    discovery: &DISCOVERY,
};
