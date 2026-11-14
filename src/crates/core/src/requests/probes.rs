//! Places where another crate's tests may hold core between two of its
//! steps, or read everything it was told. Compiled only with the `probes`
//! feature, which no build of the app turns on.
//!
//! These are the second review of ticket 27's (`evidence/27/`,
//! `reviewer2-probes.patch`). There they were two statics for the whole
//! process, told apart by a request's id and the address of its requests.
//! Tests that run side by side replay the same recordings and so share
//! request ids, and an address is used again once its owner is gone: one
//! test's answer waited at another's gate, and read another's receipts.
//! So each `Requests` holds its own.
use super::Requests;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

/// Entered and released, in that order.
pub type Gate = Arc<(AtomicBool, AtomicBool)>;

#[derive(Default)]
pub(super) struct Probes {
    accept: Mutex<Option<(String, Gate)>>,
    notes: Mutex<Vec<(String, String)>>,
}

impl Probes {
    /// Called where `answer` has taken the decision and let go of its
    /// lock, and has not yet handed the decision to the adapter.
    pub(super) fn accepted(&self, id: &str) {
        let held = self.accept.lock().unwrap();
        let gate = held.as_ref().filter(|(request, _)| request == id);
        let Some(gate) = gate.map(|(_, gate)| gate.clone()) else {
            return;
        };
        drop(held);
        gate.0.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(12);
        while !gate.1.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn noted(&self, id: &str, state: &str) {
        let row = (id.to_owned(), state.to_owned());
        self.notes.lock().unwrap().push(row);
    }
}

impl Requests {
    /// Holds `answer` for the request named here between taking its
    /// decision and handing it over: the gate's first flag says it is
    /// there, the second lets it go on. None holds nothing.
    pub fn reviewer27_hold_acceptance(&self, held: Option<(String, Gate)>) {
        *self.0.probes.accept.lock().unwrap() = held;
    }

    /// The state of every receipt these requests were told of for `id`,
    /// in order, before a later one for the same request replaced it.
    pub fn reviewer27_notes(&self, id: &str) -> Vec<String> {
        let notes = self.0.probes.notes.lock().unwrap();
        let of = |(request, _): &&(String, String)| request == id;
        notes.iter().filter(of).map(|row| row.1.clone()).collect()
    }
}
