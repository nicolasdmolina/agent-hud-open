//! Turns that start and end in a far machine's sessions, for the alerts
//! and for nothing else. This machine's hooks say when a turn starts and
//! ends. A far machine has none, a session the list says is idle is no
//! turn that ended, and the polls read only sessions that work, wait or
//! show a card. So a far gateway's polls also look at every other listed
//! session: one call that says where its stream is and returns no event.
//!
//! The model keeps the place it last saw each listed session's stream
//! at, from those looks and from what the polls read anyway. When a
//! stream has moved past its place, the poll reads on from there, and
//! each `message.start` and `message.complete` found after the place is
//! told, once: read after a place this connection saw earlier, it
//! happened since.
//!
//! Nothing is told without that. Not what is in a stream when a session
//! is first seen, or seen again after it left the list. Not anything in
//! a new stream (another epoch). Not from a place another connection
//! set: every place is forgotten when the socket is lost and when the
//! gateway restarts. And a stream that moved is no evidence by itself:
//! where Hermes no longer keeps an event, that event is not told.
//!
//! What is told carries no time. Hermes's events have none, and when an
//! event was read is not when its turn ended: a look can fail for a long
//! while and the end be read much later.
//!
//! No card rests on any of this. The places are read by nothing in
//! `reconcile.rs` or `read.rs`, the looks are steps of their own
//! (`Step::Watch`), and what is told leaves by its own door
//! (`Gateway::take_turns`), not among the effects.
use super::frame::Cursor;
use super::identity::{GatewayId, SessionRef};
use super::machine::Machine;
use super::poll::Poll;
use super::state::Gateway;
use serde_json::json;
use std::collections::BTreeMap;

mod watch;
pub(super) use watch::Watch;

/// The most turns kept for whoever takes them.
const KEPT: usize = 1_024;

/// A turn of a far session that was read to start, or to end, after a
/// place its stream had been seen at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub session: SessionRef,
    /// The stream the event was read in. Hermes counts a session's turns
    /// within one run of the gateway, so the count alone can come again.
    pub epoch: String,
    /// Hermes's count of the session's turns (`execution_generation`).
    pub generation: u64,
    /// Whether it ended; else it started.
    pub ended: bool,
}

impl Turn {
    /// A name no other turn of the session has.
    pub fn id(&self) -> String {
        format!("{}:{}", self.epoch, self.generation)
    }

    /// The fact as this machine's hooks give it for a session of its own
    /// (`hermes/hooks.rs`): a turn under way, or a turn that ended.
    /// `seen_ms` is when the poll that read the event came back. It is
    /// when the turn's start or end was seen, never when it happened:
    /// nothing Hermes sends says that. `machine` is the far machine's
    /// label, for a row made from this fact alone.
    pub fn hook_event(&self, seen_ms: i64, machine: Option<&str>) -> crate::HookEvent {
        let turn = self.id();
        let (kind, mut data) = match self.ended {
            true => (
                "completion",
                json!({"turnId": turn, "workspace": null, "model": null}),
            ),
            // No start time: when it was seen to have started is not it.
            false => (
                "session-observation",
                json!({"turnId": turn, "state": "running", "startedAtMs": null,
                    "observedAtMs": seen_ms, "workspace": null, "outcome": null,
                    "navigationTarget": null}),
            ),
        };
        if let Some(machine) = machine {
            data["machine"] = json!(machine);
        }
        crate::HookEvent {
            source: super::super::AGENT.id.to_owned(),
            session_id: self.session.hud_id(),
            at_ms: seen_ms,
            kind: kind.to_owned(),
            data,
        }
    }
}

/// What the model keeps for this between polls.
#[derive(Debug, Default)]
pub(super) struct Turns {
    /// Whether the polls look: only a far machine's.
    on: bool,
    /// Where each listed session's stream was last seen on this connection.
    seen: BTreeMap<SessionRef, Cursor>,
    /// The turns read and not yet taken.
    told: Vec<Turn>,
}

impl Turns {
    pub(super) fn of(gateway: &GatewayId) -> Self {
        let far = matches!(gateway.machine(), Machine::Far(_));
        #[cfg(test)]
        let far = forced().unwrap_or(far);
        Self {
            on: far,
            ..Self::default()
        }
    }
    /// The socket is lost or the gateway restarted: what happens until a
    /// stream is seen again is not known to have happened since.
    pub(super) fn forget(&mut self) {
        self.seen.clear();
    }
    /// What a poll that begins now takes with it.
    pub(super) fn watch(&self) -> Option<Watch> {
        self.on.then(|| Watch {
            before: self.seen.clone(),
            streams: BTreeMap::new(),
        })
    }
}

/// What the environment of a test run says of the looks, for every
/// gateway whatever its machine: the tests of the cards run both ways
/// (`tests/turns.rs`, `positions_decide_no_card`).
#[cfg(test)]
fn forced() -> Option<bool> {
    static SAID: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *SAID.get_or_init(|| match std::env::var(LOOKS).as_deref() {
        Ok("on") => Some(true),
        Ok("off") => Some(false),
        _ => None,
    })
}
#[cfg(test)]
pub(super) const LOOKS: &str = "AGENT_HUD_TEST_LOOKS";

impl Gateway {
    /// The turns read to start or end since this was last asked, in the
    /// order they happened in each session. Empty for a gateway of this
    /// machine, whose hooks tell its turns.
    pub fn take_turns(&mut self) -> Vec<Turn> {
        std::mem::take(&mut self.turns.told)
    }

    /// Takes what a poll that counts learned of the streams: each turn
    /// event read after the place its stream was last seen at is told,
    /// and the place moves to where the reads reached. The place is the
    /// one known now, not the one the poll began with: a reading of the
    /// session apart may have told part of it meanwhile.
    pub(super) fn watched(&mut self, poll: &Poll) {
        let Some(watch) = &poll.turns else {
            return;
        };
        for (at, stream) in &watch.streams {
            let Some(epoch) = &stream.epoch else {
                continue;
            };
            let known = self.turns.seen.get_mut(at);
            match known.filter(|seen| seen.epoch == *epoch && !stream.replaced) {
                Some(seen) => {
                    let Some(through) = stream.through else {
                        continue;
                    };
                    let after = stream.events.range(seen.seq.saturating_add(1)..);
                    for (_, &(generation, ended)) in after {
                        let (session, epoch) = (at.clone(), epoch.clone());
                        self.turns.told.push(Turn {
                            session,
                            epoch,
                            generation,
                            ended,
                        });
                    }
                    seen.seq = seen.seq.max(through);
                }
                // First seen, or another stream: what it holds was there.
                None => {
                    let Some(seq) = stream.through.or(stream.looked) else {
                        continue;
                    };
                    let epoch = epoch.clone();
                    self.turns.seen.insert(at.clone(), Cursor { epoch, seq });
                }
            }
        }
        let extra = self.turns.told.len().saturating_sub(KEPT);
        self.turns.told.drain(..extra);
    }

    /// A session that left the live list has no place: listed again, it
    /// is seen for the first time.
    pub(super) fn keep_watched(&mut self) {
        let listed = &self.listed;
        self.turns.seen.retain(|at, _| listed.contains_key(at));
    }
}
