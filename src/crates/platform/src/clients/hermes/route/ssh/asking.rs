//! What the route asks of the far machine over its link, and what a
//! silence there means to the connection.
use super::{Ssh, HOMES_FOR};
use crate::{
    clients::hermes::{
        control::Silent,
        far::{self, NoPicture},
    },
    ssh_link::{Failure as LinkFailure, Link},
};
use agent_hud_agents::hermes::gateway::far::{Homes, Quiet, Unchecked};
use std::{
    sync::{Arc, PoisonError},
    time::Instant,
};

impl Ssh {
    /// The link, where its master still runs. A command given to a master
    /// that is gone never starts; one that is under way when the master
    /// goes may still run on the far side, and its home is then not asked
    /// again before its deadline (`far.rs`). So a link that is known to
    /// be gone is not handed a command at all, and no home waits for one
    /// that never ran.
    pub(super) fn standing_link(&self) -> Option<(Arc<Link>, u64)> {
        let standing = |(link, _): &(Arc<Link>, u64)| link.running().is_ok();
        self.keeper.link().filter(standing)
    }

    /// The far homes, read anew where the last reading is old or of
    /// another link.
    pub(super) fn told(&self, link: &Link, number: u64) -> Result<Homes, Silent> {
        let fresh =
            |(of, when, _): &&(u64, Instant, Homes)| *of == number && when.elapsed() < HOMES_FOR;
        let lock = || self.told.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, homes)) = lock().as_ref().filter(fresh) {
            return Ok(homes.clone());
        }
        let _inside = self.gate.enter();
        match self.far.homes(link) {
            Ok(homes) => {
                *lock() = Some((number, Instant::now(), homes.clone()));
                Ok(homes)
            }
            // Not a machine to follow, as it answers now: looked at whole.
            Err(NoPicture::NotCovered(_)) => {
                self.notice.doubt();
                Err(Silent::Unclear)
            }
            Err(NoPicture::Unreached(_)) => Err(Silent::Unclear),
        }
    }

    /// What a relay's silence is to the connection (`silence`). A check
    /// that fails for one home uncovers the machine, once a look at all
    /// of it says the same.
    pub(super) fn quiet(&self, silent: far::Silent) -> Silent {
        if let far::Silent::Relay(Quiet::Unchecked(why)) = silent {
            if why != Unchecked::VerbNotSent {
                self.notice.doubt();
            }
        }
        silence(silent)
    }
}

/// What a relay's silence is to the connection. Only the relay's own
/// word, over a link that stands, says that nobody listens.
pub(super) fn silence(silent: far::Silent) -> Silent {
    match silent {
        far::Silent::Relay(Quiet::Absent) => Silent::Absent,
        far::Silent::Relay(Quiet::Unchecked(why)) => Silent::Unsafe(why.reason_code()),
        far::Silent::Relay(Quiet::TimedOut) | far::Silent::CutOff | far::Silent::Busy => {
            Silent::TimedOut
        }
        far::Silent::Relay(Quiet::Empty | Quiet::TooLong | Quiet::Garbled | Quiet::Failed) => {
            Silent::Garbled
        }
        // The link went under the relay, which says nothing of the
        // gateway: something may be there, and was not heard.
        far::Silent::Link(LinkFailure::Lost(_)) => Silent::TimedOut,
        far::Silent::Link(LinkFailure::Refused(_)) => Silent::Unclear,
    }
}

impl Ssh {
    /// Keeps the keeper from looking at its master until `blind` is
    /// false again: a test sees what is asked before it has looked.
    #[cfg(test)]
    pub(crate) fn blind_keeper(&self, blind: bool) {
        self.keeper.blind(blind);
    }
}
