//! What the routes tell the book, each from its own thread: what a route
//! verified as, that it verified as nothing, and that the machine it
//! owned is not where it leads any more.
use super::book::{Book, Lead};
use crate::clients::hermes::machine::{Inner, Keeper, Role, Standing};
use agent_hud_agents::hermes::gateway::MachineKey;

impl Keeper for Book {
    fn verified(&self, route: &Inner, key: &MachineKey) -> Role {
        let mut state = self.state();
        let Some(at) = state.slot_of(route).filter(|_| state.active) else {
            return Role::Over;
        };
        let id = state.slots[at].entry.id.clone();
        // It leads to this machine, and so to no other: a machine that
        // waited on it is retired before this one is made.
        self.unclaim(&mut state, &id, Some(key));
        let role = if self.here().as_ref() == Some(key) {
            state.slots[at].lead = Lead::Here;
            Role::Alias
        } else {
            let role = self.claim(&mut state, at, route, key);
            state.bind(at, key);
            role
        };
        drop(state);
        self.tell();
        role
    }

    fn unverified(&self, route: &Inner, standing: &Standing) {
        let mut state = self.state();
        let Some(at) = state.slot_of(route) else {
            return;
        };
        // Reached, and it is no machine: not the one that waits on it.
        // A route that cannot be reached, or whose Hermes cannot say who
        // it is just now, is not a different machine.
        if super::words::refused(standing) {
            let id = state.slots[at].entry.id.clone();
            state.slots[at].lead = Lead::Nowhere;
            self.unclaim(&mut state, &id, None);
        }
        drop(state);
        self.tell();
    }

    fn left(&self, route: &Inner, key: &MachineKey) {
        let mut state = self.state();
        let Some(at) = state.slot_of(route) else {
            return;
        };
        let id = state.slots[at].entry.id.clone();
        state.slots[at].lead = Lead::Nowhere;
        if state
            .far
            .get(key)
            .is_some_and(|far| far.owner.as_deref() == Some(&id))
        {
            self.orphan(&mut state, key);
        }
        self.unclaim(&mut state, &id, None);
        self.settle(&mut state, key);
        drop(state);
        self.tell();
    }

    fn changed(&self) {
        self.tell();
    }

    #[cfg(test)]
    fn followers(
        &self,
        route: &Inner,
    ) -> Option<std::sync::Arc<crate::clients::hermes::followers::Followers>> {
        let entry = {
            let state = self.state();
            let at = state.slot_of(route)?;
            state.slots[at].entry.id.clone()
        };
        self.followers_of(&entry)
    }
}
