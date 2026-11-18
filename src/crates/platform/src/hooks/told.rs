//! What a native channel says in a hook's place. A client's hooks report
//! a turn that starts and one that ends, on this machine. A channel that
//! follows the client where no hook of this machine runs (a far machine)
//! reads those facts itself and says them here (`Told`), as the very
//! event the client's hook would have made. They wait here, in order,
//! and the channel's wake asks for a turn of the collector, which hands
//! them to core with what the hooks reported
//! (`Hooks::drain_events`).
//!
//! Nothing is written for them: a fact told here is gone at a relaunch,
//! and none is told again after one.
use super::{Hooks, Wake};
use agent_hud_agents::HookEvent;
use std::sync::{Arc, Mutex, PoisonError};

/// A native channel's word of one fact a hook would have reported. It
/// names no client. It never waits on core: it may be called by whoever
/// read the fact, with its own state locked.
pub type Told = Arc<dyn Fn(HookEvent) + Send + Sync>;

/// The most facts kept for a collector that does not come.
const MOST: usize = 4_096;

/// The facts told and not yet handed to core, in the order told.
#[derive(Clone, Default)]
pub(super) struct Heard(Arc<Mutex<Vec<HookEvent>>>);

impl Heard {
    /// The callback a channel is given: it keeps the fact, then wakes the
    /// collector with `wake`.
    pub(super) fn teller(&self, wake: Wake) -> Told {
        let kept = self.0.clone();
        Arc::new(move |event: HookEvent| {
            let mut heard = kept.lock().unwrap_or_else(PoisonError::into_inner);
            if heard.len() < MOST {
                heard.push(event);
            }
            drop(heard);
            wake();
        })
    }

    /// Every fact told since the last call.
    pub(super) fn drain(&self) -> Vec<HookEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Hooks {
    /// The callback native channels are given.
    pub fn told(&self) -> Told {
        self.heard.teller(self.wake.clone())
    }

    /// Starts Hermes's native channel, with what it tells core beside
    /// its requests: the files it wrote (`changed.rs`), and the turns of
    /// the machines no hook of this one reports.
    pub(super) fn follow_hermes(&mut self) -> Result<(), String> {
        let (requests, wake) = (self.requests.clone(), self.wake.clone());
        let tells = (self.written.teller(wake.clone()), self.told());
        let machines = crate::clients::hermes::Machines::start(&self.data, requests, wake, tells);
        self.hermes = Some(machines?);
        Ok(())
    }

    /// Adds what the channels told to what the hooks reported. A turn a
    /// hook says started, waits or ended is a reason to ask its client's
    /// native channel what waits now. What a channel told is not: the
    /// channel read it itself, in the poll that asked for this turn.
    pub(super) fn with_told(&self, events: &mut Vec<HookEvent>) {
        let its = agent_hud_agents::hermes::AGENT.id;
        let nudge = joined(events, self.heard.drain(), its);
        if let Some(hermes) = self.hermes.as_ref().filter(|_| nudge) {
            hermes.nudge();
        }
    }
}

/// Adds what the channels `told` to what the hooks `reported`, and says
/// whether the hooks' own word names `source`: only that is a reason to
/// ask its channel again.
fn joined(reported: &mut Vec<HookEvent>, told: Vec<HookEvent>, source: &str) -> bool {
    let named = reported.iter().any(|event| event.source == source);
    reported.extend(told);
    named
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fact(kind: &str, session: &str, at_ms: i64) -> HookEvent {
        HookEvent {
            source: "cursor".into(),
            session_id: session.into(),
            at_ms,
            kind: kind.into(),
            data: serde_json::json!({"turnId": "epoch-1:1"}),
        }
    }

    /// Every fact told reaches the collector once, in the order told,
    /// and each asks for a turn of it. Two facts of one kind for one
    /// session are two facts: nothing keeps only the newest.
    #[test]
    fn what_a_channel_told_is_handed_over_once_in_order_and_wakes() {
        let woken = Arc::new(AtomicUsize::new(0));
        let count = woken.clone();
        let wake: Wake = Arc::new(move || {
            count.fetch_add(1, Ordering::AcqRel);
        });
        let heard = Heard::default();
        let told = heard.teller(wake);
        told(fact("session-observation", "far-1", 5));
        told(fact("completion", "far-1", 5));
        told(fact("completion", "far-1", 5));
        assert_eq!(woken.load(Ordering::Acquire), 3);
        let kinds: Vec<String> = heard.drain().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["session-observation", "completion", "completion"]);
        assert!(heard.drain().is_empty());
    }

    /// What a channel told is no reason to ask that channel again: it
    /// read the fact itself, and a poll for every turn told would be a
    /// poll too many each time. A hook's own word of the client is one.
    #[test]
    fn what_a_channel_told_asks_for_no_poll_and_a_hooks_word_does() {
        let told = || {
            vec![
                fact("session-observation", "far-1", 5),
                fact("completion", "far-1", 5),
            ]
        };
        let mut events = Vec::new();
        assert!(!joined(&mut events, told(), "cursor"));
        assert_eq!(events.len(), 2);
        // A hook of another client beside them: still none.
        let mut events = vec![HookEvent {
            source: "copilot".into(),
            ..fact("completion", "near-1", 4)
        }];
        assert!(!joined(&mut events, told(), "cursor"));
        assert_eq!(events.len(), 3);
        // The client's own hook reported: its channel is asked.
        let mut events = vec![fact("completion", "near-1", 4)];
        assert!(joined(&mut events, told(), "cursor"));
        assert!(joined(&mut events, Vec::new(), "cursor"));
    }

    /// A collector that never comes costs a bounded queue.
    #[test]
    fn a_queue_nobody_drains_stops_growing() {
        let heard = Heard::default();
        let told = heard.teller(Arc::new(|| {}));
        for at in 0..(MOST as i64 + 50) {
            told(fact("completion", "far-1", at));
        }
        assert_eq!(heard.drain().len(), MOST);
    }
}
