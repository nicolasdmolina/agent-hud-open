//! Hermes's native channel (`agents::hermes::GATEWAY`): the local gateway
//! its desktop app talks to. This is the half with the sockets. A machine
//! can run one gateway for all of its profiles or one for each, so every
//! Hermes home its route tells of is asked who is there (`discovery.rs`),
//! and each gateway found has a follower of its own (`followers.rs`,
//! `follower.rs`). A follower gets a ticket from the control socket of
//! its gateway's home, opens the WebSocket on a stream the route gives,
//! and runs the polls that `agents::hermes::gateway` begins. What that
//! model says to do is done here and nothing else is decided: a card
//! opens, changes or closes in core's requests, and an answer given on a
//! card goes back on a socket of its own.
//!
//! It reads Hermes and never writes under a Hermes home. It takes no part
//! in collection: it never holds the collection gate, and every wait on
//! a gateway, writes included, happens with nothing locked. An answer
//! is never written once its follower ended or its card closed: a write is
//! checked and marked under the model's lock, and ending or closing cuts
//! the socket of a write that is under way (`visit.rs`).
use crate::hooks::Wake;
use agent_hud_agents::hermes::gateway::Machine;
use agent_hud_core::requests::Requests;
use std::{path::Path, sync::Arc};

mod answer;
mod apart;
mod asking;
mod control;
mod cost;
mod decisions;
mod discovery;
mod effects;
pub mod far;
pub mod far_history;
mod follower;
mod followers;
mod link;
mod log;
mod machine;
mod model;
mod opening;
mod out;
mod pace;
mod polls;
mod route;
mod settle;
mod signal;
mod socket;
#[cfg(test)]
mod tap;
mod visit;
mod wire;

use follower::Shared;
pub use machine::{FarEntry, FarObserver, Standing};
use model::Model;
pub(crate) use pace::Pace;
pub(crate) use route::Route;

pub struct Observer {
    all: Arc<followers::Followers>,
}

impl Observer {
    /// Follows the gateways of this machine's Hermes home, logging into
    /// `data`.
    pub fn start(data: &Path, requests: Requests, wake: Wake) -> Result<Self, String> {
        let route = Arc::new(route::local::Local::new()?);
        Self::start_on(route, data, requests, wake, Pace::default())
    }

    /// Follows the gateways of the machine `route` leads to.
    pub(crate) fn start_on(
        route: Arc<dyn Route>,
        data: &Path,
        requests: Requests,
        wake: Wake,
        pace: Pace,
    ) -> Result<Self, String> {
        let this = (Machine::This, None);
        let all = followers::Followers::start(route, this, data, requests, wake, pace)?;
        Ok(Self { all })
    }

    /// A Hermes hook reported a turn started, waiting or ended: poll now.
    pub fn nudge(&self) {
        self.all.nudge();
    }

    /// The followers there are now, for the tests that look at one.
    #[cfg(test)]
    fn followers(&self) -> Vec<Arc<Shared>> {
        self.all.set().clone()
    }

    /// The follower of this machine's own gateway, as it is now.
    #[cfg(test)]
    fn local(&self) -> Arc<Shared> {
        let own = |follower: &Arc<Shared>| follower.reader == "local";
        self.followers().into_iter().find(own).unwrap()
    }

    /// Every reader ended in core so far, in order.
    #[cfg(test)]
    fn ends(&self) -> Vec<String> {
        self.all.ends.lock().unwrap().clone()
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.all.stop();
    }
}

#[cfg(all(test, unix))]
mod tests_unix;
