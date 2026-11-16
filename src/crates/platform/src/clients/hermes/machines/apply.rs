//! Taking a connection list that parsed. Only such a list changes which
//! entries there are: an entry is removed or pointed elsewhere by a list
//! that parsed and no longer holds it, or holds it changed. A list that
//! is equal in every field that is read changes nothing, whatever else
//! in the file changed: the desktop app rewrites it whenever its own
//! token changes.
use super::book::{Book, Lead, Slot, State};
use crate::{clients::hermes::machine::Inner, ssh_link::Target};
use agent_hud_agents::hermes::connections::{Entry, Kind, SshAddress};
use std::{collections::BTreeSet, path::Path, sync::Arc};

/// What became of an entry the list had before.
enum Was {
    /// Gone from the list, with where its route led.
    Removed(String, Lead),
    /// Its address was edited: its route is another, under a new number.
    Readdressed(String, Lead),
    Renamed(String, String),
}

impl Book {
    /// The list as it parsed just now.
    pub(crate) fn apply(&self, list: Vec<Entry>) {
        let mut state = self.state();
        if !state.active {
            return;
        }
        state.unread = None;
        if state.list.as_ref() == Some(&list) {
            return self.tell_with(&state);
        }
        let mut before = std::mem::take(&mut state.slots);
        let (mut was, mut stops) = (Vec::new(), Vec::new());
        let mut listed = BTreeSet::new();
        for entry in &list {
            // A second entry of one id is not another entry.
            if !listed.insert(entry.id.clone()) {
                continue;
            }
            let had = before.iter().position(|slot| slot.entry.id == entry.id);
            let slot = match had.map(|at| before.remove(at)) {
                Some(mut slot) if slot.entry.kind == entry.kind => {
                    if slot.entry.label != entry.label {
                        was.push(Was::Renamed(entry.id.clone(), entry.label.clone()));
                    }
                    slot.entry = entry.clone();
                    slot
                }
                Some(slot) => {
                    was.push(Was::Readdressed(entry.id.clone(), slot.lead));
                    stops.extend(slot.route);
                    let number = slot.number + 1;
                    self.log.line(&format!("entry readdressed number={number}"));
                    self.slot(entry, number)
                }
                None => self.slot(entry, 1),
            };
            state.slots.push(slot);
        }
        for slot in before {
            was.push(Was::Removed(slot.entry.id, slot.lead));
            stops.extend(slot.route);
        }
        for was in was {
            self.became(&mut state, was);
        }
        state.reorder();
        state.bindings.keep(&|entry| listed.contains(entry));
        state.list = Some(list);
        self.tell_with(&state);
        drop(state);
        #[cfg(test)]
        self.before_teardown();
        // With nothing locked: closing a link waits for its master.
        for route in stops {
            route.stop();
        }
    }

    /// One entry given directly, with no list: its route.
    pub(in crate::clients::hermes) fn given(
        &self,
        label: &str,
        target: Target,
    ) -> Result<Arc<Inner>, String> {
        let address = SshAddress {
            host: target.destination(),
            user: None,
            port: target.port(),
            key_path: None,
        };
        let (id, label) = ("given".to_owned(), label.to_owned());
        let kind = Kind::Ssh(address);
        let entry = Entry { id, label, kind };
        // Locked while it starts: the route's first word finds its slot.
        let mut state = self.state();
        let route = self.route(target)?;
        let mut slot = Slot::of(entry, 1);
        slot.route = Some(route.clone());
        state.slots.push(slot);
        Ok(route)
    }

    /// The slot of `entry`, with its route started where it has one.
    fn slot(&self, entry: &Entry, number: u64) -> Slot {
        let mut slot = Slot::of(entry.clone(), number);
        let Kind::Ssh(address) = &entry.kind else {
            return slot;
        };
        let key = address.key_path.as_deref().map(Path::new);
        let target = Target::new(&address.host, address.user.as_deref(), address.port, key);
        let route = target
            .map_err(|refused| refused.reason().to_owned())
            .and_then(|target| self.route(target));
        match route {
            Ok(route) => slot.route = Some(route),
            Err(why) => slot.without = Some(why),
        }
        slot
    }

    fn route(&self, target: Target) -> Result<Arc<Inner>, String> {
        let ways = self.ways.as_ref().map_err(|why| (*why).to_owned())?;
        let ways = (ways.folder.clone(), ways.settings.clone());
        Inner::start(target, ways, self.keeper(), (&self.data, self.pace))
    }

    fn became(&self, state: &mut State, was: Was) {
        let routed = |state: &State, id: &str| {
            let its = |slot: &Slot| slot.entry.id == id && slot.route.is_some();
            state.slots.iter().any(its)
        };
        match was {
            // The machine waits on the entry's new route, where it has
            // one, and on every other entry that reached it before.
            Was::Readdressed(id, Lead::Owner(key)) => {
                self.orphan(state, &key);
                let routed = routed(state, &id);
                if let Some(far) = state.far.get_mut(&key).filter(|_| routed) {
                    far.pending.insert(id);
                }
                self.settle(state, &key);
            }
            // With no other entry that reaches it, the machine is
            // retired: its last entry is gone.
            Was::Removed(_, Lead::Owner(key)) => {
                self.orphan(state, &key);
                self.settle(state, &key);
            }
            // A machine that waits on this entry waits on its newest
            // route: an address edited twice is no word that it leads
            // elsewhere. An entry with no route leads nowhere.
            Was::Readdressed(id, _) if routed(state, &id) => {}
            Was::Readdressed(id, _) | Was::Removed(id, _) => self.unclaim(state, &id, None),
            Was::Renamed(id, label) => self.relabel(state, &id, &label),
        }
    }
}

impl Slot {
    fn of(entry: Entry, number: u64) -> Self {
        Self {
            entry,
            number,
            route: None,
            without: None,
            lead: Lead::Nowhere,
        }
    }
}

impl State {
    /// The first entry in the list's order owns: an alias that comes
    /// before its machine's owner now is asked to verify, and takes the
    /// machine when it has.
    fn reorder(&self) {
        for (key, far) in &self.far {
            let Some(owner) = &far.owner else {
                continue;
            };
            for slot in &self.slots {
                if slot.entry.id == *owner {
                    break;
                }
                if slot.lead == Lead::Alias(key.clone()) {
                    if let Some(route) = &slot.route {
                        route.stir();
                    }
                }
            }
        }
    }
}
