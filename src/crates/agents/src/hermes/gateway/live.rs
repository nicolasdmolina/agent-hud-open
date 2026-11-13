//! What the gateway says each live session is doing, for the HUD to show
//! over the hooks' guess. The hooks report a turn's start and end; between
//! them nothing says a long tool call still runs or a question still waits,
//! so core lets a quiet turn lapse to idle after two minutes. The gateway's
//! list says it outright, at every poll.
//!
//! Only what is current is told: nothing while the gateway is not ready,
//! and only the sessions of profiles the newest poll listed. A profile
//! whose list failed, or a gateway that went away, falls back to the hooks
//! rather than show a status nobody has read since.
use super::state::Gateway;
use super::Connection;
use crate::LiveState;

impl Gateway {
    /// Every live session the HUD can trust a status for, by its HUD
    /// session id. A session with a card here waits, whatever Hermes lists
    /// (it says `working` for a set of questions). Empty when the gateway
    /// is not ready or has not been polled since it last was.
    pub fn live_states(&self) -> Vec<(String, LiveState)> {
        if !matches!(self.connection, Connection::Ready(_)) {
            return Vec::new();
        }
        self.statuses()
            .into_iter()
            .filter(|(at, _)| self.fresh.contains(&at.profile))
            .map(|(at, status)| (at.hud_id(), status.live()))
            .collect()
    }
}
