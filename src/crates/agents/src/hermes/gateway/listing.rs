//! The live list as rows. `session.active_list` says more of a live
//! session than what it is doing: its title, its model, where it was
//! started and when. That is enough for a row, and it is all a far
//! machine's row is made from until its history is read: nothing here
//! says what a session used.
//!
//! Only what is current is given, by the rule of `live.rs`: the profiles
//! the newest poll that counted listed, while the gateway is ready. A
//! profile whose list failed is left out, since what it holds now is not
//! known; one that was listed with no session is there, empty.
use super::identity::Profile;
use super::sample::Sample;
use super::state::Gateway;
use super::{Connection, SessionRef, Status};
use serde_json::Value;
use std::collections::BTreeMap;

/// The longest title taken from a list, and the longest of its other
/// words. What a gateway says is not trusted to be short.
const TITLE: usize = 512;
const WORD: usize = 128;

/// What a row of one live session shows, as the gateway's list gave it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listed {
    pub title: Option<String>,
    pub model: Option<String>,
    /// Where Hermes says it was started: `telegram`, `cli`, `cron`.
    pub source: Option<String>,
    /// When it started, in Hermes's seconds. None where the list gives
    /// no time.
    pub started_at: Option<f64>,
}

impl Listed {
    /// What one row of `session.active_list` says.
    pub(super) fn of(row: &Value) -> Self {
        let word = |key: &str, most: usize| {
            let said = crate::text(&row[key])?;
            Some(said.chars().take(most).collect::<String>())
        };
        let time = |at: f64| at.is_finite() && at > 0.0;
        Self {
            title: word("title", TITLE),
            model: word("model", WORD),
            source: word("source", WORD),
            started_at: row["started_at"].as_f64().filter(|at| time(*at)),
        }
    }
}

/// The live sessions of a gateway as one poll listed them.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// When the poll that listed them came back.
    pub read_ms: i64,
    /// Each profile that poll listed, with its sessions by Hermes's id.
    pub profiles: BTreeMap<Profile, Vec<(String, Listed)>>,
    /// What that poll's samples said its sessions have used so far, for
    /// showing alone (`sample.rs`). Only a far gateway's poll takes any.
    pub samples: BTreeMap<Profile, Vec<(String, Sample)>>,
}

impl Gateway {
    /// The live sessions the newest poll that counted listed, profile by
    /// profile. None while the gateway is not ready, or has not been
    /// polled since it last was.
    pub fn listing(&self) -> Option<Listing> {
        let ready = matches!(self.connection, Connection::Ready(_));
        let fresh = self.told.fresh();
        if !ready || fresh.is_empty() {
            return None;
        }
        let mut profiles: BTreeMap<Profile, Vec<(String, Listed)>> =
            (fresh.iter().cloned().map(|profile| (profile, Vec::new()))).collect();
        for at in self.listed.keys() {
            if let (Some(of), Some(row)) = (profiles.get_mut(&at.profile), self.rows.get(at)) {
                of.push((at.id().to_owned(), row.clone()));
            }
        }
        let read_ms = self.told.read_ms();
        // Taken by that same poll, so of sessions it listed and no others.
        let samples = self.told.samples().clone();
        Some(Listing {
            read_ms,
            profiles,
            samples,
        })
    }

    /// Whether a session of that listing is in a turn: starting or
    /// working. One that is idle is not, however many are listed, and
    /// neither is one that waits for an answer, whose counts stand still
    /// until it is given: also where Hermes lists it as working and no
    /// card shows here (ticket 63). A far machine's history is read at its
    /// quick pace only while this holds (ticket 61).
    pub fn working(&self) -> bool {
        let ready = matches!(self.connection, Connection::Ready(_));
        let fresh = self.told.fresh();
        let at_work = |at: &SessionRef| {
            let turn = matches!(self.status(at), Some(Status::Starting | Status::Working));
            let waits = self.asking.contains(at);
            turn && !waits && fresh.iter().any(|profile| *profile == at.profile)
        };
        ready && self.listed.keys().any(at_work)
    }
}
