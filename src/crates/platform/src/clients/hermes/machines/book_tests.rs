//! What the tests read of the book, and the one place they hold it.
use super::book::{Book, Far, Slot};
use crate::clients::hermes::{
    far_history::Slots, followers::Followers, history::History, machine::Inner,
};
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

    /// The history refresh of the machine the entry `entry` owns.
    pub(in crate::clients::hermes) fn history_of(&self, entry: &str) -> Option<Arc<History>> {
        let state = self.state();
        let owned = |far: &&Far| far.owner.as_deref() == Some(entry);
        (state.far.values().find(owned)).and_then(|far| far.history.clone())
    }

    /// The route switch of the machine the entry `entry` owns.
    pub(in crate::clients::hermes) fn switch_of(
        &self,
        entry: &str,
    ) -> Option<Arc<crate::clients::hermes::route::switch::Switch>> {
        let state = self.state();
        let owned = |far: &&Far| far.owner.as_deref() == Some(entry);
        (state.far.values().find(owned)).map(|far| far.switch.clone())
    }

    /// The history readers' slots, one for each far machine.
    pub(in crate::clients::hermes) fn slots(&self) -> Arc<Slots> {
        self.slots.clone()
    }

    /// The followers of the machine the entry `entry` owns.
    pub(in crate::clients::hermes) fn followers_of(&self, entry: &str) -> Option<Arc<Followers>> {
        let state = self.state();
        let owned = |far: &&Far| far.owner.as_deref() == Some(entry);
        (state.far.values().find(owned)).map(|far| far.followers.clone())
    }
}

impl super::Machines {
    /// Hermes followed in a lab: this machine's gateway is looked for in
    /// `folder` alone, and there is no connection list and no far route.
    /// Nothing of the user's own is read.
    pub(crate) fn in_a_lab(
        folder: &std::path::Path,
        requests: agent_hud_core::requests::Requests,
    ) -> Self {
        let sources = super::Sources {
            local: Arc::new(crate::clients::hermes::route::local::Local::at(
                folder.join("hermes"),
            )),
            here: None,
            list: None,
            ways: Err("no far machine in a lab"),
            look: std::time::Duration::from_millis(40),
            pace: crate::clients::hermes::Pace::default(),
            changed: Arc::new(|_| {}),
            told: Arc::new(|_| {}),
        };
        let data = folder.join("data");
        crate::os::private_directory(&data).expect("the lab's data folder");
        Self::start_with(sources, &data, requests, Arc::new(|| {})).expect("the machines")
    }

    /// How often Hermes's own gateways were asked to be polled now.
    pub(crate) fn nudges(&self) -> u64 {
        (self.nudged).load(std::sync::atomic::Ordering::Acquire)
    }
}
