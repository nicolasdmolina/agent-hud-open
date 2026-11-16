//! What the tests read of the book, and the one place they hold it.
use super::book::{Book, Far, Slot};
use crate::clients::hermes::{followers::Followers, machine::Inner};
use std::sync::Arc;

/// Entered, and let go on.
#[derive(Default)]
pub(in crate::clients::hermes) struct Gate {
    pub(in crate::clients::hermes) entered: std::sync::atomic::AtomicBool,
    pub(in crate::clients::hermes) released: std::sync::atomic::AtomicBool,
}

impl Book {
    /// Waits where a test holds the list that was just taken.
    pub(super) fn before_teardown(&self) {
        use std::sync::atomic::Ordering;
        let Some(gate) = self.teardown.lock().unwrap().take() else {
            return;
        };
        gate.entered.store(true, Ordering::Release);
        let limit = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !gate.released.load(Ordering::Acquire) && std::time::Instant::now() < limit {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// What the tests read of the book: the far machines there are, each
    /// with the entry that owns it and its route number.
    pub(in crate::clients::hermes) fn far(&self) -> Vec<(String, Option<String>, u64)> {
        let state = self.state();
        let all = state.far.iter();
        all.map(|(key, far)| {
            (
                key.as_str().to_owned(),
                far.owner.clone(),
                far.switch.number(),
            )
        })
        .collect()
    }

    /// The route of the entry `entry`, as it is now.
    pub(in crate::clients::hermes) fn route_of(&self, entry: &str) -> Option<Arc<Inner>> {
        let state = self.state();
        let its = |slot: &&Slot| slot.entry.id == entry;
        state
            .slots
            .iter()
            .find(its)
            .and_then(|slot| slot.route.clone())
    }

    /// The followers of the machine the entry `entry` owns.
    pub(in crate::clients::hermes) fn followers_of(&self, entry: &str) -> Option<Arc<Followers>> {
        let state = self.state();
        let owned = |far: &&Far| far.owner.as_deref() == Some(entry);
        (state.far.values().find(owned)).map(|far| far.followers.clone())
    }
}
