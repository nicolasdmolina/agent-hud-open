//! Which far machines there are, and which entry's route owns each.
//!
//! **Entries, routes and machines are three things.** An entry is a row
//! of the connection list. A route is one connection for an entry
//! (`machine.rs`), with a number that goes up when the entry's address
//! is edited. A machine is a verified identity, a key: its followers,
//! their cards and their readers in core are the machine's, and are
//! kept here by that key, never by an entry or a route.
//!
//! **One owner.** At most one entry's route owns a machine: the first
//! entry in the list's order whose route verified as it. Every other
//! entry that verifies as the same machine is an alias and follows
//! nothing, and so does one that verifies as the machine Agent HUD runs
//! on.
//!
//! Every change is one step under the book's lock, which is never held
//! across a wait on the network: a route is told to stop once the lock
//! is let go. What a route change does to a machine is in `change.rs`.
use super::{bindings::Bindings, list::Unread};
use crate::{
    clients::hermes::{
        followers::Followers,
        log::Log,
        machine::{Inner, Keeper, Role},
        mirror::Writer,
        route::{switch::Switch, Route},
        Pace,
    },
    hooks::{Changed, Told, Wake},
    ssh_link::{Folder, Settings},
};
use agent_hud_agents::hermes::{
    connections::{Entry, Kind},
    gateway::{Machine, MachineKey},
};
use agent_hud_core::requests::Requests;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
};

/// How a route is made: the folder that holds every master, and how
/// `ssh` is run.
pub(crate) struct Ways {
    pub(crate) folder: Folder,
    pub(crate) settings: Settings,
}

/// Says which machine Agent HUD runs on, where its Hermes names one.
pub(crate) type Here = Arc<dyn Fn() -> Option<MachineKey> + Send + Sync>;

pub(crate) struct Book {
    pub(super) data: PathBuf,
    pub(super) requests: Requests,
    wake: Wake,
    /// Tells core which files of a machine's mirrors changed.
    changed: Changed,
    /// Tells core the turns that start and end on a far machine.
    told: Told,
    pub(super) pace: Pace,
    pub(super) log: Log,
    /// None where no route can be made, with the word for why.
    pub(super) ways: Result<Ways, &'static str>,
    here: Option<Here>,
    me: Weak<Book>,
    state: Mutex<State>,
    /// Holds a list that was taken between its locked step and the
    /// stopping of the routes it ended, for the tests of that moment.
    #[cfg(test)]
    pub(in crate::clients::hermes) teardown: Mutex<Option<Arc<super::book_tests::Gate>>>,
}

pub(super) struct State {
    pub(super) active: bool,
    /// The list as it last parsed. None until one did.
    pub(super) list: Option<Vec<Entry>>,
    /// One for each entry, in the list's order.
    pub(super) slots: Vec<Slot>,
    pub(super) far: BTreeMap<MachineKey, Far>,
    /// Why the list cannot be read, while it cannot.
    pub(super) unread: Option<Unread>,
    pub(super) bindings: Bindings,
}

pub(super) struct Slot {
    pub(super) entry: Entry,
    /// The entry's route number: one for each address it has had.
    pub(super) number: u64,
    pub(super) route: Option<Arc<Inner>>,
    /// Why an entry that could have a route has none.
    pub(super) without: Option<String>,
    pub(super) lead: Lead,
}

/// Where an entry's route leads, as it last verified. An alias whose
/// address was edited is still its machine's alias until its new route
/// verifies, or is reached and refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Lead {
    Nowhere,
    /// To the machine Agent HUD runs on.
    Here,
    Owner(MachineKey),
    Alias(MachineKey),
}

/// One far machine that is followed.
pub(super) struct Far {
    pub(super) switch: Arc<Switch>,
    pub(super) followers: Arc<Followers>,
    /// The entry whose route is behind the switch. None while the route
    /// changes: then nothing is asked of the machine, and its cards wait
    /// marked.
    pub(super) owner: Option<String>,
    pub(super) label: String,
    /// While it has no owner: the entries whose routes may yet verify as
    /// it. With none left, the machine is retired.
    pub(super) pending: BTreeSet<String>,
}

impl Book {
    pub(crate) fn start(
        (data, pace): (&Path, Pace),
        requests: Requests,
        (wake, changed, told): (Wake, Changed, Told),
        ways: Result<Ways, &'static str>,
        here: Option<Here>,
        bindings: Bindings,
    ) -> Arc<Self> {
        let state = State {
            active: true,
            list: None,
            slots: Vec::new(),
            far: BTreeMap::new(),
            unread: None,
            bindings,
        };
        Arc::new_cyclic(|me| Self {
            data: data.to_owned(),
            requests,
            wake,
            changed,
            told,
            pace,
            log: Log::new(data).of(Some("machines")),
            ways,
            here,
            me: me.clone(),
            state: Mutex::new(state),
            #[cfg(test)]
            teardown: Mutex::default(),
        })
    }

    pub(super) fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The book as the routes know it.
    pub(super) fn keeper(&self) -> Weak<dyn Keeper> {
        self.me.clone()
    }

    /// The key of the machine Agent HUD runs on, as it is now.
    pub(super) fn here(&self) -> Option<MachineKey> {
        self.here.as_ref().and_then(|here| here())
    }

    /// Ends every machine and every route: cards withdrawn, readers
    /// ended, links closed. No master is left when this returns.
    pub(crate) fn stop(&self) {
        let (far, routes) = {
            let mut state = self.state();
            if !std::mem::replace(&mut state.active, false) {
                return;
            }
            let routes: Vec<Arc<Inner>> = (state.slots.iter_mut())
                .filter_map(|slot| slot.route.take())
                .collect();
            (std::mem::take(&mut state.far), routes)
        };
        for far in far.into_values() {
            far.followers.stop();
        }
        for route in routes {
            route.stop();
        }
        // Each route and each machine has said that it stopped.
        self.log.line("off");
        self.tell();
    }

    /// Makes the machine `key`, which the route of the slot at `at`
    /// verified as, or says what that route is to it.
    pub(super) fn claim(
        &self,
        state: &mut State,
        at: usize,
        route: &Inner,
        key: &MachineKey,
    ) -> Role {
        let (id, label) = {
            let entry = &state.slots[at].entry;
            (entry.id.clone(), entry.label.clone())
        };
        let way: Arc<dyn Route> = route.route.clone();
        let owner = state.far.get(key).map(|far| far.owner.clone());
        match owner {
            None => {
                let switch = Arc::new(Switch::new(way));
                let machine = (Machine::Far(key.clone()), Some(label.clone()));
                let (requests, wake) = (self.requests.clone(), self.wake.clone());
                // The machine's one mirror writer, made with it and its
                // for as long as it is followed. Without one, the
                // machine has its cards and no rows.
                let log = Log::new(&self.data).of(Some(key.as_str()));
                let (told, routes) = (self.changed.clone(), switch.clone());
                let mirror = Writer::start(&self.data, (key, &label), routes, told, log);
                if let Err(why) = &mirror {
                    self.log
                        .line(&format!("no mirror machine={} {why}", key.as_str()));
                }
                let Ok(followers) = Followers::start(
                    switch.clone(),
                    machine,
                    &self.data,
                    requests,
                    wake,
                    self.pace,
                    (mirror.ok(), Some(self.told.clone())),
                ) else {
                    return Role::Alias;
                };
                let far = Far {
                    switch,
                    followers,
                    owner: Some(id),
                    label,
                    pending: BTreeSet::new(),
                };
                state.far.insert(key.clone(), far);
            }
            // The machine waited for an owner: this route is it.
            Some(None) => self.install(state, key, (&id, &label), way),
            Some(Some(owner)) if owner == id => {}
            // The first in the list's order owns: a route that verified
            // later, for an entry that comes sooner, takes the machine.
            Some(Some(owner)) if state.precedes(&id, &owner) => {
                self.orphan(state, key);
                self.install(state, key, (&id, &label), way);
            }
            Some(Some(_)) => {
                state.slots[at].lead = Lead::Alias(key.clone());
                return Role::Alias;
            }
        }
        state.slots[at].lead = Lead::Owner(key.clone());
        Role::Owner
    }
}

impl State {
    /// Remembers that the entry of the slot at `at` verified as `key`.
    pub(super) fn bind(&mut self, at: usize, key: &MachineKey) {
        let entry = &self.slots[at].entry;
        if let Kind::Ssh(address) = &entry.kind {
            (self.bindings).record(&entry.id, address, key, &entry.label);
        }
    }

    /// The slot whose route is `route`, the very one.
    pub(super) fn slot_of(&self, route: &Inner) -> Option<usize> {
        let its = |slot: &Slot| (slot.route.as_deref()).is_some_and(|own| std::ptr::eq(own, route));
        self.slots.iter().position(its)
    }

    /// Whether the entry `first` comes before `second` in the list.
    pub(super) fn precedes(&self, first: &str, second: &str) -> bool {
        let at = |id: &str| self.slots.iter().position(|slot| slot.entry.id == id);
        match (at(first), at(second)) {
            (Some(first), Some(second)) => first < second,
            (first, _) => first.is_some(),
        }
    }
}
