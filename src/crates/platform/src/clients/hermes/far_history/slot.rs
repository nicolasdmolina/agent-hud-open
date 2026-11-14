//! One history reader at a time for each far machine, on this side.
//!
//! The slot belongs to the machine, by its key. It outlives a link, a
//! route and a refresh, and [`Slots`] keeps it for as long as the app
//! runs, so a machine that is retired and added again gets the slot it
//! had. It says what is known here of a reader on the far side:
//!
//! - free: none runs;
//! - held until a time: a reader's `started` was seen here at a moment,
//!   and its answer did not end as an answer does. The reader counts its
//!   deadline from when it wrote `started`, which was no later than when
//!   it was seen, so it is gone by that moment plus the deadline and a
//!   margin;
//! - held, start unknown: a reader may have been started and nothing says
//!   when. It may not have begun yet, so no countdown here frees this. It
//!   is freed only by a new reader that starts, which a reader does only
//!   when no other one is alive, and then ends.
//!
//! Nothing here knows what happened before this process began, so a slot
//! starts as held, start unknown.
use agent_hud_agents::hermes::gateway::MachineKey;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

/// What a slot knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    Free,
    Until(Instant),
    StartUnknown,
}

pub struct Slot {
    /// One read at a time: held from before a reader starts until its
    /// answer has ended on this side.
    turn: Mutex<()>,
    state: Mutex<State>,
}

struct State {
    hold: Hold,
    /// Goes up when the machine is retired. What a reader started before
    /// that learns afterwards no longer changes the slot.
    round: u64,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Slot {
    fn new() -> Self {
        Self {
            turn: Mutex::new(()),
            state: Mutex::new(State {
                hold: Hold::StartUnknown,
                round: 0,
            }),
        }
    }

    pub fn hold(&self) -> Hold {
        locked(&self.state).hold
    }

    pub(super) fn turn(&self) -> MutexGuard<'_, ()> {
        locked(&self.turn)
    }

    /// A reader is about to be started: until its `started` is seen,
    /// nothing says when it starts. The round to report under, and what
    /// the slot said before.
    pub(super) fn begin(&self) -> (u64, Hold) {
        let mut state = locked(&self.state);
        let before = std::mem::replace(&mut state.hold, Hold::StartUnknown);
        (state.round, before)
    }

    /// No reader was started after all: the slot says what it said.
    pub(super) fn restore(&self, round: u64, before: Hold) {
        self.set(round, before);
    }

    fn set(&self, round: u64, hold: Hold) {
        let mut state = locked(&self.state);
        if state.round == round {
            state.hold = hold;
        }
    }

    /// The reader's `started` was seen: it is gone by `until`.
    pub(super) fn started(&self, round: u64, until: Instant) {
        self.set(round, Hold::Until(until));
    }

    /// The far process is known to be gone, or never was.
    pub(super) fn ended(&self, round: u64) {
        self.set(round, Hold::Free);
    }

    /// Another reader is alive there, started nobody here knows when.
    pub(super) fn busy(&self, round: u64) {
        self.set(round, Hold::StartUnknown);
    }

    /// The time a cut-off reader was held until has passed.
    pub(super) fn lapsed(&self, until: Instant) {
        let mut state = locked(&self.state);
        if state.hold == Hold::Until(until) && Instant::now() >= until {
            state.hold = Hold::Free;
        }
    }

    fn retired(&self) {
        let mut state = locked(&self.state);
        state.round += 1;
        if state.hold != Hold::Free {
            state.hold = Hold::StartUnknown;
        }
    }
}

/// Every far machine's slot, for as long as the app runs.
#[derive(Default)]
pub struct Slots {
    all: Mutex<HashMap<MachineKey, Arc<Slot>>>,
}

impl Slots {
    pub fn new() -> Self {
        Self::default()
    }

    /// The machine's one slot. A machine first met here gets one that is
    /// held, start unknown.
    pub fn of(&self, machine: &MachineKey) -> Arc<Slot> {
        let mut all = locked(&self.all);
        let slot = all
            .entry(machine.clone())
            .or_insert_with(|| Arc::new(Slot::new()));
        Arc::clone(slot)
    }

    /// The machine was retired. If it comes back it has this slot still,
    /// and where a reader was not known to be gone, nothing is known of
    /// it any more: the next read there starts with a probe.
    pub fn retired(&self, machine: &MachineKey) {
        if let Some(slot) = locked(&self.all).get(machine) {
            slot.retired();
        }
    }
}
