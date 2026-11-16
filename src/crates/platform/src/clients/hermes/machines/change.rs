//! What a route change does to a machine, in this order:
//!
//! 1. The machine's route number goes up, before anything waits or is
//!    torn down (`cut`). From that step no result of the old route is
//!    taken and no answer is sent on it (`route/switch.rs`).
//! 2. In the same step the machine's cards are marked as out of reach,
//!    and stay registered: nothing is settled and nothing is registered
//!    again (`handover.rs`).
//! 3. A route verifies its identity: the entry's new one, or that of
//!    another entry that reached the same machine before, which is asked
//!    to verify anew (`orphan`).
//! 4. The same machine: it goes on under that owner (`install`). Its
//!    followers dial over the new way, and the first poll reconciles
//!    the cards.
//! 5. Another machine, or an identity that is refused: that entry leads
//!    there no more (`unclaim`). With no entry left that may, the machine
//!    is retired (`retire`): its cards are withdrawn and its readers
//!    end. Only then is the new machine made, if there is one.
//!
//! A route that cannot be reached is not another machine: the machine
//! waits for it, and nothing is retired.
use super::book::{Book, Far, Lead, State};
use crate::clients::hermes::route::Route;
use agent_hud_agents::hermes::gateway::MachineKey;
use std::sync::Arc;

impl Book {
    /// Steps 1 and 2 for the machine `key`.
    fn cut(&self, far: &Far, key: &MachineKey) {
        let number = far.switch.raise();
        far.followers.lose();
        let line = format!("route change machine={} number={number}", key.as_str());
        self.log.line(&line);
    }

    /// The machine `key` has no owner from now on: its route changes,
    /// and every entry that verified as it before is asked to verify
    /// anew, the machine waiting on each of them.
    pub(super) fn orphan(&self, state: &mut State, key: &MachineKey) {
        let Some(far) = state.far.get_mut(key) else {
            return;
        };
        self.cut(far, key);
        if let Some(owner) = far.owner.take() {
            if let Some(slot) = state.slots.iter_mut().find(|slot| slot.entry.id == owner) {
                // Still its entry's route: another route took the machine.
                if slot.lead == Lead::Owner(key.clone()) {
                    slot.lead = Lead::Alias(key.clone());
                }
            }
        }
        for slot in &state.slots {
            if slot.lead != Lead::Alias(key.clone()) {
                continue;
            }
            if let Some(route) = &slot.route {
                far.pending.insert(slot.entry.id.clone());
                route.stir();
            }
        }
    }

    /// Step 4: the machine `key` goes on under the entry `id`, over `way`.
    pub(super) fn install(
        &self,
        state: &mut State,
        key: &MachineKey,
        (id, label): (&str, &str),
        way: Arc<dyn Route>,
    ) {
        let Some(far) = state.far.get_mut(key) else {
            return;
        };
        far.switch.install(way);
        far.owner = Some(id.to_owned());
        far.pending.clear();
        far.label = label.to_owned();
        far.followers.relabel(label);
        far.followers.stir();
        let line = format!(
            "owner machine={} number={}",
            key.as_str(),
            far.switch.number()
        );
        self.log.line(&line);
    }

    /// The entry `id` leads to no machine but `keep`: one that waited on
    /// it waits no more, and is retired where it waited on nobody else.
    pub(super) fn unclaim(&self, state: &mut State, id: &str, keep: Option<&MachineKey>) {
        let waited: Vec<MachineKey> = (state.far.iter_mut())
            .filter(|(key, _)| Some(*key) != keep)
            .filter_map(|(key, far)| far.pending.remove(id).then(|| key.clone()))
            .collect();
        for key in waited {
            self.settle(state, &key);
        }
    }

    /// Retires the machine `key` where it has no owner and no entry left
    /// that may yet lead to it.
    pub(super) fn settle(&self, state: &mut State, key: &MachineKey) {
        let over = |far: &Far| far.owner.is_none() && far.pending.is_empty();
        if state.far.get(key).is_some_and(over) {
            self.retire(state, key);
        }
    }

    /// Step 5: the machine `key` is followed no more. Its cards are
    /// withdrawn and its readers end, which is authoritative: no entry
    /// leads to it.
    fn retire(&self, state: &mut State, key: &MachineKey) {
        let Some(far) = state.far.remove(key) else {
            return;
        };
        far.followers.stop();
        for slot in &mut state.slots {
            if matches!(&slot.lead, Lead::Owner(of) | Lead::Alias(of) if of == key) {
                slot.lead = Lead::Nowhere;
            }
        }
        self.log.line(&format!("retired machine={}", key.as_str()));
    }

    /// The entry `id` is called `label` from now on: so is the machine
    /// it owns, on every card that shows.
    pub(super) fn relabel(&self, state: &mut State, id: &str, label: &str) {
        let owned = |far: &&mut Far| far.owner.as_deref() == Some(id);
        if let Some(far) = state.far.values_mut().find(owned) {
            far.label = label.to_owned();
            far.followers.relabel(label);
        }
        state.bindings.relabel(id, label);
    }
}
