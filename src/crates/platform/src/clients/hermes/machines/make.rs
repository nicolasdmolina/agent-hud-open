//! Making a far machine, and ending one: what is the machine's own, and
//! so made once with it and kept through every route change. Its
//! followers and their cards (`followers.rs`), the one writer of its
//! mirrors (`mirror.rs`), and what keeps those mirrors in step with its
//! Hermes databases (`history.rs`).
use super::book::{Book, Far};
use crate::clients::hermes::{
    followers::Followers,
    history::History,
    log::Log,
    mirror::Writer,
    route::{switch::Switch, Route},
};
use agent_hud_agents::hermes::gateway::{Machine, MachineKey};
use std::{collections::BTreeSet, sync::Arc};

impl Book {
    /// The machine `key`, owned by the entry `id` that is called `label`
    /// and reached by `way`. None where its followers could not start.
    pub(super) fn make(
        &self,
        key: &MachineKey,
        (id, label): (String, String),
        way: Arc<dyn Route>,
    ) -> Option<Far> {
        let switch = Arc::new(Switch::new(way));
        let machine = (Machine::Far(key.clone()), Some(label.clone()));
        let (requests, wake) = (self.requests.clone(), self.wake.clone());
        // The machine's one mirror writer, made with it and its for as
        // long as it is followed. Without one, the machine has its cards
        // and no rows.
        let log = || Log::new(&self.data).of(Some(key.as_str()));
        let (told, routes) = (self.changed.clone(), switch.clone());
        let mirror = Writer::start(&self.data, (key, &label), routes, told, log());
        if let Err(why) = &mirror {
            self.log
                .line(&format!("no mirror machine={} {why}", key.as_str()));
        }
        let mirror = mirror.ok();
        let followers = Followers::start(
            switch.clone(),
            machine,
            &self.data,
            requests,
            wake,
            self.pace,
            (mirror.clone(), Some(self.told.clone())),
        )
        .ok()?;
        // Its stored history, read into the mirrors that writer writes.
        // The slot is asked of the book's own, never made anew.
        let history = mirror.map(|writer| {
            let (slots, plan) = (self.slots.clone(), self.pace.history);
            History::start(
                &self.data,
                key,
                switch.clone(),
                writer,
                slots,
                (plan, log()),
            )
        });
        Some(Far {
            switch,
            followers,
            history,
            owner: Some(id),
            label,
            pending: BTreeSet::new(),
        })
    }
}

impl Far {
    /// The machine is followed no more: no history read begins, its
    /// followers end with their cards and readers, and its mirrors are
    /// written no more.
    pub(super) fn stop(&self) {
        if let Some(history) = &self.history {
            history.stop();
        }
        self.followers.stop();
    }
}
