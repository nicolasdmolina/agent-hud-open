//! The route to the gateways of a far machine, over SSH (`ssh_link`).
//! Nothing that follows a gateway changes for it: the same followers, the
//! same polls and the same answers run over it as over the route to this
//! machine.
//!
//! - **Homes.** The far login shell's Hermes root and its profile folders,
//!   resolved there (`far.rs`). The list is never taken for whole: the
//!   listing leaves out a folder it could not enter and does not say so,
//!   so a home that is missing from it is not known to be gone.
//! - **The control socket.** One relay for each exchange, which asks as
//!   the login user and only where the home, the socket and its listener
//!   are that user's own (`far/relay.py`). No far socket is forwarded.
//! - **The stream.** A port of this machine forwarded to the port of an
//!   instance the model's `identify()` accepted, which has checked that
//!   its origin is loopback there. The port is taken from that instance
//!   and from nowhere else. The forward is made in `prepare`, so it is in
//!   place before the ticket is asked for, and one serves every socket to
//!   its gateway (`forwards.rs`).
//! - **Who answers.** The first socket to each gateway is asked for the
//!   install's own id, and nothing is told of a gateway whose answer is
//!   not the machine's (`identity.rs`, `admit`).
//!
//! **A lost link is never word that a gateway is gone.** With no link, the
//! homes cannot be told and a control socket did not answer in time; only
//! the relay's own word, over a link that stands, says that nobody listens
//! there. The link is opened again by its keeper, for as long as it takes
//! (`keeper.rs`); the followers go on asking, as they do for a gateway
//! that is stopped, and find their gateways again over the new link, with
//! new forwards.
use super::{Call, Home, Inventory, Route, Stop};
use crate::{
    clients::hermes::{control::Silent, far::Far},
    ssh_link::{Failure as LinkFailure, Folder, Link, Settings, Target},
};
use agent_hud_agents::hermes::gateway::{
    far::{FarHome, Homes, Request},
    profiles_list, reported_install, Failure, Instance, MachineKey, Profile, Scope,
};
use serde_json::Value;
use std::{
    borrow::Cow,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

mod asking;
mod first;
mod forwards;
mod identity;
mod keeper;
mod notice;
#[cfg(test)]
mod tests;

pub(crate) use first::First;
pub(crate) use identity::NoMachine;
pub(crate) use keeper::Says;
pub(crate) use notice::Noticed;

/// How long a dial through a forward may take: the listener is this
/// machine's own `ssh`.
const DIAL: Duration = Duration::from_secs(3);
/// How long one reading of the far homes is used. Every ticket asks which
/// home it is asked in, and a reading is a command on the far machine.
const HOMES_FOR: Duration = Duration::from_secs(5);

pub(crate) struct Ssh {
    keeper: keeper::Keeper,
    far: Far,
    forwards: Arc<forwards::Forwards>,
    identity: identity::Identity,
    notice: notice::Notice,
    gate: notice::Gate,
    /// The last reading of the far homes: the link it was read on, when.
    told: Mutex<Option<(u64, Instant, Homes)>>,
    /// What was asked of the route, in order, for the tests of that order.
    #[cfg(test)]
    steps: Mutex<Vec<(&'static str, std::thread::ThreadId)>>,
}

impl Ssh {
    /// The route to the machine at `target`, with its masters in `folder`.
    /// The link is opened in the background and kept open from now on.
    pub(crate) fn start(
        folder: Folder,
        target: Target,
        settings: Settings,
        says: Says,
    ) -> Result<Self, String> {
        Ok(Self {
            keeper: keeper::Keeper::start(folder, target, settings, says)?,
            far: Far::default(),
            forwards: Arc::default(),
            identity: identity::Identity::default(),
            notice: notice::Notice::default(),
            gate: notice::Gate::default(),
            told: Mutex::default(),
            #[cfg(test)]
            steps: Mutex::default(),
        })
    }

    /// Notes one step asked of the route, by the thread that asked.
    #[cfg(test)]
    fn step(&self, step: &'static str) {
        let asked = (step, std::thread::current().id());
        self.steps.lock().unwrap().push(asked);
    }
    #[cfg(not(test))]
    fn step(&self, _step: &'static str) {}

    /// Every step asked so far, each with the thread that asked it.
    #[cfg(test)]
    pub(crate) fn all_steps(&self) -> Vec<(&'static str, std::thread::ThreadId)> {
        self.steps.lock().unwrap().clone()
    }

    /// How often `step` was asked.
    #[cfg(test)]
    pub(crate) fn steps(&self, step: &str) -> usize {
        let asked = self.all_steps();
        asked.iter().filter(|(of, _)| *of == step).count()
    }

    /// Closes the link and opens no other.
    pub(crate) fn stop(&self) {
        self.keeper.stop();
        self.notice.wake();
    }

    pub(crate) fn link(&self) -> Option<(Arc<Link>, u64)> {
        self.keeper.link()
    }

    /// The link, where its master still runs. A command given to a master
    /// that is gone never starts; one that is under way when the master
    /// goes may still run on the far side, and its home is then not asked
    /// again before its deadline (`far.rs`). So a link that is known to
    /// be gone is not handed a command at all, and no home waits for one
    /// that never ran.
    fn standing_link(&self) -> Option<(Arc<Link>, u64)> {
        self.keeper
            .link()
            .filter(|(link, _)| link.running().is_ok())
    }

    /// Why there is no link, where one failed to open or was lost.
    pub(crate) fn down(&self) -> Option<LinkFailure> {
        self.keeper.down()
    }

    pub(crate) fn far(&self) -> &Far {
        &self.far
    }

    /// How many forwards were ever made.
    #[cfg(test)]
    pub(crate) fn forwards_made(&self) -> u64 {
        self.forwards.made()
    }

    /// Waits until the route says something of its machine.
    pub(crate) fn noticed(&self, over: &dyn Fn() -> bool) -> Noticed {
        self.notice.wait(over)
    }

    pub(crate) fn uncover(&self, why: &str) {
        self.notice.uncover(why);
    }

    /// The machine is verified: the gateways that reported, and the root.
    pub(crate) fn settle(
        &self,
        root: &str,
        reported: &[(String, Option<String>)],
        link: u64,
    ) -> Result<MachineKey, NoMachine> {
        self.notice.clear();
        self.identity.settle(root, reported, link)
    }

    /// The route leads to no machine until it is verified again, and no
    /// gateway of it is followed: every forward is cancelled.
    pub(crate) fn forget(&self) {
        self.identity.forget();
        self.notice.clear();
        self.forwards.release_all(self.keeper.link());
    }
}

/// The port a forward to `instance` leads to: the one its origin names,
/// where that origin is plain `http://127.0.0.1:<port>`, the only far
/// address a forward ever leads to.
fn far_port(instance: &Instance) -> Option<u16> {
    let port = instance.origin.strip_prefix("http://127.0.0.1:")?;
    let port: u16 = port.parse().ok()?;
    (port != 0 && port == instance.port).then_some(port)
}

impl Route for Ssh {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.inventory().map(|told| told.homes)
    }

    fn inventory(&self) -> Result<Inventory, Silent> {
        let (link, number) = self.standing_link().ok_or(Silent::Unclear)?;
        let told = self.told(&link, number)?;
        if !self.identity.rooted_at(&told.root) {
            self.notice.uncover(NoMachine::RootMoved.reason_code());
            return Err(Silent::Unclear);
        }
        let home = |far: &FarHome| {
            let path = PathBuf::from(&far.path);
            Home::at(path.clone(), path, far.profile.clone())
        };
        let homes = told.homes.iter().map(home).collect();
        Ok(Inventory {
            homes,
            whole: false,
        })
    }

    fn exchange(&self, home: &Home, line: &[u8], _timeout: Duration) -> Result<Vec<u8>, Silent> {
        // The relay carries its own deadline, which is the wait here.
        let asked: Value = serde_json::from_slice(line).map_err(|_| Silent::Unsafe("request"))?;
        let request = Request::of(&asked).ok_or(Silent::Unsafe("request"))?;
        self.step(match request {
            Request::Identify => "identify",
            Request::Ticket { .. } => "ticket",
        });
        let path = home.path().to_str().ok_or(Silent::Unsafe("home"))?;
        let (link, _) = self.standing_link().ok_or(Silent::TimedOut)?;
        let _inside = self.gate.enter();
        match self.far.exchange(&link, path, request).said {
            Ok(answered) => {
                let mut answer = answered.value().to_string().into_bytes();
                answer.push(b'\n');
                Ok(answer)
            }
            Err(silent) => Err(self.quiet(silent)),
        }
    }

    /// A far home is named as the far side resolved it. The relay asks a
    /// home only by that name: one that leads elsewhere by then is
    /// refused there.
    fn named(&self, home: &Home) -> Cow<'_, str> {
        Cow::Owned(home.path().to_string_lossy().into_owned())
    }

    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        let port = far_port(instance).ok_or(Failure::Invalid("invalid_api_origin"))?;
        self.step("prepare");
        let (link, number) = self.keeper.link().ok_or(Failure::Unreachable)?;
        self.forwards.prepare(&link, number, instance, port, stop)
    }

    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.step("dial");
        let (_, number) = self.keeper.link().ok_or(Failure::Unreachable)?;
        let local = self.forwards.local(number, &instance.id);
        let local = local.ok_or(Failure::Unreachable)?;
        let at = SocketAddr::from((Ipv4Addr::LOCALHOST, local));
        TcpStream::connect_timeout(&at, DIAL).map_err(|_| Failure::Unreachable)
    }

    fn release(&self, instance: &str) {
        self.forwards.release(self.keeper.link(), instance);
    }

    fn admit(&self, instance: &Instance, call: Call<'_>) -> Result<(), Failure> {
        let (_, number) = self.keeper.link().ok_or(Failure::Unreachable)?;
        if self.identity.admitted(number, &instance.id) {
            return Ok(());
        }
        let result = call(&profiles_list(&asked_profile(instance), Scope::Host))?;
        let reported = reported_install(&result);
        self.identity
            .admit(number, &instance.id, reported)
            .map_err(|why| {
                self.notice.uncover(why.reason_code());
                Failure::Refused(format!("not the machine followed: {}", why.reason_code()))
            })
    }
}

/// The profile a call on a socket to `instance` names: the one whose home
/// was asked, as its gateway names it.
fn asked_profile(instance: &Instance) -> Profile {
    let asked = |served: &&agent_hud_agents::hermes::gateway::Served| {
        served.profile_id == instance.profile_id
    };
    let served = instance.served.iter().find(asked);
    served.map_or(Profile::Default, |served| served.profile.clone())
}
