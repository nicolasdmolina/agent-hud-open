//! What the gateway says each live session is doing, for the HUD to show
//! over the hooks' guess. The hooks report a turn's start and end; between
//! them nothing says a long tool call still runs or a question still waits,
//! so core lets a quiet turn lapse to idle after two minutes. The gateway's
//! list says it outright, at every poll.
//!
//! Only what is current is told: nothing while the gateway is not ready,
//! and only the sessions of profiles the newest poll listed. A profile
//! whose list failed, or a gateway that went away, falls back to the hooks
//! rather than show a status nobody has read since. What is told carries
//! when the poll that read it came back, and nothing else renews it: core
//! stops believing it once that is too long ago, so a channel that hangs
//! without a word gives the sessions back to the hooks all the same.
use super::poll::Poll;
use super::state::Gateway;
use super::{Connection, Profile};
use crate::LiveStates;
use std::collections::BTreeSet;

/// What the HUD can be told rests on: the profiles the newest poll that
/// counted listed since the gateway was last ready, whose sessions in
/// `listed` are current, and when that poll came back.
#[derive(Debug, Default)]
pub(super) struct Told {
    fresh: BTreeSet<Profile>,
    read_ms: i64,
}

impl Told {
    /// A gateway that is not ready lists nothing current.
    pub(super) fn follow(&mut self, next: &Connection) {
        if !matches!(next, Connection::Ready(_)) {
            self.fresh.clear();
        }
    }
    /// A poll that counted came back at `now_ms`: the profiles it listed
    /// are current, and no others.
    pub(super) fn read(&mut self, poll: &Poll, now_ms: i64) {
        let listed = poll.profiles.iter().filter(|(_, l)| **l == Some(true));
        self.fresh = listed.map(|(profile, _)| profile.clone()).collect();
        self.read_ms = now_ms;
    }
}

impl Gateway {
    /// Every live session the HUD can trust a status for, by its HUD
    /// session id. A session with a card here waits, whatever Hermes lists
    /// (it says `working` for a set of questions). Empty when the gateway
    /// is not ready or has not been polled since it last was. `read_ms`
    /// is when the newest poll that counted came back.
    pub fn live_states(&self) -> LiveStates {
        if !matches!(self.connection, Connection::Ready(_)) {
            return LiveStates::default();
        }
        let sessions = self
            .statuses()
            .into_iter()
            .filter(|(at, _)| self.told.fresh.contains(at.route()))
            .map(|(at, status)| (at.hud_id(), status.live()))
            .collect();
        LiveStates {
            sessions,
            read_ms: self.told.read_ms,
        }
    }
}
