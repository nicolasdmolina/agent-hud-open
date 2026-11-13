//! Hermes.
pub mod account;
pub mod gateway;
mod hooks;
mod session_counts;
mod settings;
pub mod transcript;

use crate::Agent;
use crate::Discovery;
use crate::{Carries, Channel, PerOs, Support};
use crate::{Location, Source};

/// Hermes's local gateway, which its desktop app talks to: the questions
/// and approvals a session waits on, and the answers to them. Agent HUD
/// asks the gateway's control socket, as the same user, for a ticket that
/// opens one WebSocket (`gateway`). A gateway without this connection is
/// followed by the hooks alone.
pub static GATEWAY: Channel = Channel {
    name: "Hermes's local gateway",
    carries: &[Carries::Requests],
    support: PerOs {
        macos: Support::Verified(()),
        windows: Support::Unsupported(
            "Hermes's gateway is asked through a named pipe on Windows, which is not implemented",
        ),
        linux: Support::Unverified(()),
    },
};

static SOURCES: [Source; 1] = [Source {
    reads_as: "hermes",
    location: Location::home(&["HERMES_HOME"], &[".hermes"], ""),
}];
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
    sources: crate::everywhere(&SOURCES),
    discovery: &DISCOVERY,
};
