//! Which machine a route leads to. A far machine is known by what its own
//! Hermes says of its install, read over a socket that was opened with a
//! ticket, and by its root home as the far side resolved it. Nothing
//! stands in for that: not the address the link was opened to, and not
//! an id of the connection entry.
//!
//! Every gateway of the route must say the same. The gateways found
//! before anything is followed are asked first, and they make the key
//! (`settle`). Each gateway that turns up later, and each one again after
//! the link was lost and reopened, is asked on its first socket and held
//! against that (`admit`). One that says anything else means the route
//! leads to no one machine, and nothing more of it is told.
//!
//! The install id itself is kept here, in memory, to hold later answers
//! against. It is in no log, no id and no `Debug` output.
use agent_hud_agents::hermes::gateway::{identities_agree, machine_key, MachineKey, NoKey};
use std::{
    collections::HashSet,
    sync::{Mutex, MutexGuard, PoisonError},
};

#[derive(Default)]
pub(super) struct Identity {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    verified: Option<Verified>,
    /// The number of the link the gateways below were admitted on.
    link: u64,
    /// The instances that said who they are on that link.
    admitted: HashSet<String>,
}

struct Verified {
    install: String,
    root: String,
}

/// Why a route leads to no machine that can be followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoMachine {
    /// What Hermes there reports as its install makes no key.
    NoKey(NoKey),
    /// Two gateways of the route report different installs.
    Disagree,
    /// A gateway reports another install than the machine was known by.
    Another,
    /// The far root home is another than the machine was known by.
    RootMoved,
    /// Nothing was verified on this route, so nothing can be held against.
    Unverified,
}

impl NoMachine {
    /// A word for the log and the report.
    pub(crate) fn reason_code(self) -> &'static str {
        match self {
            Self::NoKey(why) => why.reason_code(),
            Self::Disagree => "install_ids_disagree",
            Self::Another => "install_id_changed",
            Self::RootMoved => "root_home_changed",
            Self::Unverified => "identity_unverified",
        }
    }
}

impl Identity {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The machine the route leads to, from what each gateway found on it
    /// reported on its first socket (`reported`: the instance, and its
    /// install id as it wrote it) and from the far root home. All of them
    /// must agree and make a key; then those instances are admitted on
    /// the link `link`.
    pub(super) fn settle(
        &self,
        root: &str,
        reported: &[(String, Option<String>)],
        link: u64,
    ) -> Result<MachineKey, NoMachine> {
        let ids = || reported.iter().map(|(_, id)| id.as_deref());
        let first = ids().next().ok_or(NoMachine::Unverified)?;
        if !identities_agree(ids()) {
            return Err(NoMachine::Disagree);
        }
        let key = machine_key(first, root).map_err(NoMachine::NoKey)?;
        let mut state = self.state();
        state.verified = Some(Verified {
            install: first.unwrap_or_default().to_owned(),
            root: root.to_owned(),
        });
        state.link = link;
        state.admitted = reported.iter().map(|(id, _)| id.clone()).collect();
        Ok(key)
    }

    /// Whether `instance` has said who it is on the link `link`.
    pub(super) fn admitted(&self, link: u64, instance: &str) -> bool {
        let state = self.state();
        state.verified.is_some() && state.link == link && state.admitted.contains(instance)
    }

    /// Holds what `instance` reported on its first socket on the link
    /// `link` against the machine the route was verified as.
    pub(super) fn admit(
        &self,
        link: u64,
        instance: &str,
        reported: Option<&str>,
    ) -> Result<(), NoMachine> {
        let mut state = self.state();
        let verified = state.verified.as_ref().ok_or(NoMachine::Unverified)?;
        // Read as the first was, so the same faults say the same thing.
        machine_key(reported, &verified.root).map_err(NoMachine::NoKey)?;
        if reported != Some(verified.install.as_str()) {
            return Err(NoMachine::Another);
        }
        if state.link != link {
            // Who was admitted on another link is asked again.
            state.admitted.clear();
            state.link = link;
        }
        state.admitted.insert(instance.to_owned());
        Ok(())
    }

    /// Whether `root` is the root home of the machine the route was
    /// verified as. So it is while nothing is verified yet.
    pub(super) fn rooted_at(&self, root: &str) -> bool {
        let state = self.state();
        state
            .verified
            .as_ref()
            .is_none_or(|known| known.root == root)
    }

    /// The route leads to no machine any more, until it is verified again.
    pub(super) fn forget(&self) {
        let mut state = self.state();
        state.verified = None;
        state.admitted.clear();
    }
}
