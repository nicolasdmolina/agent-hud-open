//! The machines list: one status for each entry of the connection list,
//! told to core whole (`Requests::set_machines`) whenever it may have
//! changed. Core carries it and decides nothing from it.
//!
//! This machine is always one of them, whether or not the list has an
//! entry for it. While no list has parsed since launch, the machines
//! verified before are shown from their bindings, and nothing is
//! dialled. A status goes by its machine's key once its entry verified,
//! now or before (the key its sessions' ids begin with), and by the
//! entry's id until then. One status goes by a key, the owner's: every
//! other entry that leads to the machine goes by its own id, in reach or
//! out of it, so no two statuses share an id. An entry's id that could
//! be taken for another's (a machine's key, this machine's `local`, or
//! one already written so) goes by `entry:` and that id.
use super::{
    book::{Book, Lead, Slot, State},
    list::Unread,
    words,
};
use crate::clients::hermes::{history::Ages, machine::Standing};
use agent_hud_agents::{
    hermes::{
        connections::{Kind, SshAddress},
        gateway::StoredKey,
        AGENT,
    },
    MachineState, MachineStatus, Machines as Told,
};

/// What this machine's status goes by where the list has no entry for
/// it: the id the list's own entry for it always has.
const HERE: &str = "local";

impl Book {
    /// Tells core the machines as they are now.
    pub(super) fn tell(&self) {
        self.tell_with(&self.state());
    }

    /// The same, by whoever holds the book: told in the order the book
    /// changed, never an older list after a newer one.
    pub(super) fn tell_with(&self, state: &State) {
        let told = match state.active {
            true => state.told(),
            false => Told::default(),
        };
        self.requests.set_machines(AGENT.id, &told);
    }
}

impl Book {
    /// The list could not be read, and why; or it is as it last parsed.
    /// Nothing else changes: the note on the list, and no machine.
    pub(crate) fn read(&self, unread: Option<Unread>) {
        let mut state = self.state();
        if state.unread != unread {
            state.unread = unread;
            let line = match unread {
                Some(why) => format!("list unread why={why:?}"),
                None => "list read".to_owned(),
            };
            self.log.line(&line);
        }
        self.tell_with(&state);
    }

    /// Closes every master an earlier launch left in the control folder.
    pub(crate) fn sweep(&self) {
        if let Ok(ways) = &self.ways {
            ways.folder.sweep(&ways.settings);
        }
    }
}

fn status(id: &str, label: &str, state: MachineState, note: Option<String>) -> MachineStatus {
    MachineStatus {
        id: id.to_owned(),
        label: label.to_owned(),
        state,
        note,
        contact_ms: None,
        history_checked_ms: None,
        here: false,
    }
}

/// The id an entry's status goes by while its machine is not known: the
/// entry's own, written so that it is never taken for a machine's key or
/// this machine's `local`, nor for another entry's id written so.
fn row(entry: &str) -> String {
    let taken = StoredKey::parse(entry).is_some() || entry == HERE;
    match taken || entry.starts_with(ENTRY) {
        true => format!("{ENTRY}{entry}"),
        false => entry.to_owned(),
    }
}

/// What an entry's id is written after where it could be taken for
/// another's.
const ENTRY: &str = "entry:";

impl State {
    fn told(&self) -> Told {
        let mut list = Vec::new();
        let listed = |slot: &Slot| slot.entry.kind == Kind::This;
        if !self.slots.iter().any(listed) {
            list.push(self.here_status(HERE, &words::this_machine()));
        }
        match &self.list {
            Some(_) => list.extend(self.slots.iter().map(|slot| self.status(slot))),
            // No list has parsed: what was verified before, undialled.
            None => {
                for (key, label) in self.bindings.machines() {
                    let note = Some(words::awaiting_list());
                    list.push(status(key, label, MachineState::CannotReach, note));
                }
            }
        }
        // A list that was never there, with nothing verified before, is
        // no list to speak of: Hermes's desktop app may not be in use.
        let silent =
            self.unread == Some(Unread::Missing) && self.list.is_none() && self.bindings.is_empty();
        let list_note = self.unread.filter(|_| !silent).map(words::unread);
        Told { list, list_note }
    }

    fn status(&self, slot: &Slot) -> MachineStatus {
        let entry = &slot.entry;
        match &entry.kind {
            Kind::This => self.here_status(&entry.id, &entry.label),
            Kind::NotCovered(why) => {
                let note = Some(words::kind(*why));
                status(
                    &row(&entry.id),
                    &entry.label,
                    MachineState::NotCovered,
                    note,
                )
            }
            Kind::Ssh(address) => self.far_status(slot, address),
        }
    }

    fn far_status(&self, slot: &Slot, address: &SshAddress) -> MachineStatus {
        let entry = &slot.entry;
        // Verified before at this address: it goes by that machine's key,
        // where it is the one entry that does.
        let before = self.bindings.of(&entry.id, address);
        let before = before.filter(|key| self.named(key) == Some(entry.id.as_str()));
        let known = before.map_or_else(|| row(&entry.id), str::to_owned);
        let Some(route) = &slot.route else {
            let why = slot.without.as_deref().unwrap_or_default();
            let note = Some(words::no_route(why));
            return status(&known, &entry.label, MachineState::NotCovered, note);
        };
        let mut ages = Ages::default();
        let (id, state, note) = match route.standing() {
            // The owner, but for the moment in which another entry's
            // route took the machine and this one has yet to hear of it.
            Standing::Connected(key) => {
                let named = self.named(key.as_str()).is_none_or(|id| id == entry.id);
                let id = if named {
                    key.as_str().to_owned()
                } else {
                    known
                };
                // When it was last read, and when its history was last
                // checked whole; or why it has no history.
                let history = self.far.get(&key).and_then(|far| far.history.as_ref());
                ages = history.map(|history| history.ages()).unwrap_or_default();
                let note =
                    (ages.without.map(words::no_history)).or_else(|| words::far_notes(&ages));
                (id, MachineState::Connected, note)
            }
            Standing::Alias(key) => {
                let other = match &slot.lead {
                    Lead::Here => self.here_label(),
                    _ => (self.far.get(&key)).map_or_else(String::new, |far| far.label.clone()),
                };
                (row(&entry.id), MachineState::SameMachine, Some(other))
            }
            Standing::Connecting => (known, MachineState::Connecting, None),
            Standing::NotTrusted => (known, MachineState::NotTrusted, Some(words::not_trusted())),
            Standing::CannotReach(why) => {
                let note = Some(words::cannot_reach(why));
                (known, MachineState::CannotReach, note)
            }
            Standing::NotCovered(why) => {
                (known, MachineState::NotCovered, Some(words::reason(&why)))
            }
        };
        let told = status(
            &id,
            &entry.label,
            state,
            note.filter(|note| !note.is_empty()),
        );
        MachineStatus {
            contact_ms: ages.contact_ms,
            history_checked_ms: ages.checked_ms,
            ..told
        }
    }

    /// The entry whose status goes by the machine `key`: its owner, and
    /// while it has none (it waits for one, or nothing is followed yet)
    /// the first entry in the list's order that verified as it at the
    /// address it has now.
    fn named(&self, key: &str) -> Option<&str> {
        let far = self.far.iter().find(|(far, _)| far.as_str() == key);
        let owner = far.and_then(|(_, far)| far.owner.as_deref());
        let bound = |slot: &&Slot| match &slot.entry.kind {
            Kind::Ssh(address) => self.bindings.of(&slot.entry.id, address) == Some(key),
            _ => false,
        };
        let first = || {
            self.slots
                .iter()
                .find(bound)
                .map(|slot| slot.entry.id.as_str())
        };
        owner.or_else(first)
    }

    /// This machine's status: as its own followers knew at the last look.
    fn here_status(&self, id: &str, label: &str) -> MachineStatus {
        let (state, note) = self.here.told();
        MachineStatus {
            here: true,
            ..status(id, label, state, note)
        }
    }

    /// What this machine is called: its entry's label, where it has one.
    fn here_label(&self) -> String {
        let listed = |slot: &&Slot| slot.entry.kind == Kind::This;
        let label = self
            .slots
            .iter()
            .find(listed)
            .map(|slot| slot.entry.label.clone());
        label.unwrap_or_else(words::this_machine)
    }
}

#[cfg(test)]
mod tests {
    use super::{row, HERE};

    /// No entry's id is ever this Mac's, a machine's key, or another
    /// entry's: here an entry named `local`, one named as a key is, and
    /// one named as that key's entry goes by.
    #[test]
    fn an_entry_never_goes_by_an_id_that_is_not_its_own() {
        let key = "0123456789abcdef";
        assert_eq!(row("b"), "b");
        assert_eq!(row(key), format!("entry:{key}"));
        let entries = [
            HERE.to_owned(),
            key.to_owned(),
            format!("entry:{key}"),
            "b".into(),
        ];
        let ids = entries.clone().map(|entry| row(&entry));
        for taken in [HERE, key] {
            assert!(!ids.iter().any(|id| id == taken), "{ids:?}");
        }
        let mut seen = ids.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len(), "{ids:?}");
    }
}
