//! The machines list: one status for each entry of the connection list,
//! told to core whole (`Requests::set_machines`) whenever it may have
//! changed. Core carries it and decides nothing from it.
//!
//! This machine is always one of them, whether or not the list has an
//! entry for it. While no list has parsed since launch, the machines
//! verified before are shown from their bindings, and nothing is
//! dialled. A status goes by its machine's key once its entry verified,
//! now or before (the key its sessions' ids begin with), and by the
//! entry's id until then.
use super::{
    book::{Book, Lead, Slot, State},
    list::Unread,
    words,
};
use crate::clients::hermes::machine::Standing;
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
/// entry's own, written so that it is never taken for a machine's key.
fn row(entry: &str) -> String {
    match StoredKey::parse(entry) {
        Some(_) => format!("entry:{entry}"),
        None => entry.to_owned(),
    }
}

impl State {
    fn told(&self) -> Told {
        let mut list = Vec::new();
        let listed = |slot: &Slot| slot.entry.kind == Kind::This;
        if !self.slots.iter().any(listed) {
            let here = status(HERE, &words::this_machine(), MachineState::Connected, None);
            list.push(MachineStatus { here: true, ..here });
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
            Kind::This => {
                let here = status(&entry.id, &entry.label, MachineState::Connected, None);
                MachineStatus { here: true, ..here }
            }
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
        // Verified before at this address: it goes by that machine's key.
        let before = self.bindings.of(&entry.id, address).map(str::to_owned);
        let known = before.unwrap_or_else(|| row(&entry.id));
        let Some(route) = &slot.route else {
            let why = slot.without.as_deref().unwrap_or_default();
            let note = Some(words::no_route(why));
            return status(&known, &entry.label, MachineState::NotCovered, note);
        };
        let (id, state, note) = match route.standing() {
            Standing::Connected(key) => (key.as_str().to_owned(), MachineState::Connected, None),
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
        status(
            &id,
            &entry.label,
            state,
            note.filter(|note| !note.is_empty()),
        )
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
