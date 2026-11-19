//! Keeps a far machine's mirrors in step with its Hermes databases
//! (ticket 37; spec "Phase 2 design", section 14). One of these for each
//! far machine, made with it and the machine's for as long as it is
//! followed, like its mirror writer: a route change that leaves the
//! machine an owner leaves the refresh where it was, sweep and all.
//!
//! It runs apart from the polls, on a thread of its own, and never slows
//! them: it shares the machine's link and nothing else, holds no model's
//! lock and no lock of core's, and writes through the one mirror writer.
//!
//! - **A tick** reads every Hermes home of the machine in turn, a bounded
//!   number of reads each (`tick.rs`). What each read is for, and that
//!   the whole sweep gets its share, is decided by
//!   `agents::hermes::far_history::refresh`.
//! - **One reader at a time** for the machine: every read asks the
//!   machine's one slot (`far_history::Slots`, kept by whoever keeps the
//!   machines). The refresh never makes a slot.
//! - **What is read under one route number is written under it**, or not
//!   at all: the writer drops a batch whose number is not the machine's
//!   now, and one that comes back after the machine was retired.
//! - **The interval.** A tick a minute while a far session is in a turn
//!   or a whole sweep is overdue, and every five minutes otherwise,
//!   however many idle sessions the machine lists (ticket 61). A session
//!   that takes a turn during the long wait ends it: the wait looks once
//!   a minute, on this Mac alone.
use super::{
    far_history::{Failure, Reader, Slot, Slots},
    log::Log,
    mirror::Writer,
    route::Route,
};
use agent_hud_agents::hermes::{
    far_history::{refresh, Far, No},
    gateway::{MachineKey, Profile},
    mirror::{self, Reads},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

mod plan;
mod tick;

use plan::FRESH;
pub(crate) use plan::{Ages, Plan, Ticked, Without};

pub(crate) struct History {
    key: MachineKey,
    /// The folder the mirrors are in.
    root: PathBuf,
    route: Arc<dyn Route>,
    writer: Arc<Writer>,
    reader: Reader,
    slots: Arc<Slots>,
    slot: Arc<Slot>,
    plan: Plan,
    log: Log,
    state: Mutex<State>,
    wake: Condvar,
    /// One tick at a time, and what each profile's sweeps have got to.
    progress: Mutex<HashMap<Profile, tick::Progress>>,
}

#[derive(Default)]
struct State {
    over: bool,
    /// Each profile's ages, as last read from its mirror or set by a tick.
    ages: HashMap<Profile, Reads>,
    without: Option<Without>,
    /// How many sessions of each profile the mirror refused.
    refused: HashMap<Profile, usize>,
}

impl History {
    /// The refresh of the machine `key`, whose mirrors are under `data`
    /// and written by `writer`, over `route`. `slots` are the app's, and
    /// the machine's one slot is asked of them here, once.
    pub(crate) fn start(
        data: &Path,
        key: &MachineKey,
        route: Arc<dyn Route>,
        writer: Arc<Writer>,
        slots: Arc<Slots>,
        (plan, log): (Plan, Log),
    ) -> Arc<Self> {
        let said = log.of(None);
        let history = Arc::new(Self {
            key: key.clone(),
            root: data.join(mirror::ROOT),
            route,
            writer,
            reader: Reader::new(plan.reader, move |line| said.line(line)),
            slot: slots.of(key),
            slots,
            plan,
            log,
            state: Mutex::default(),
            wake: Condvar::new(),
            progress: Mutex::default(),
        });
        if plan.clock {
            let owned = history.clone();
            let started = thread::Builder::new()
                .name("Hermes history".into())
                .spawn(move || owned.work());
            if started.is_err() {
                history.log.line("history not started");
            }
        }
        history
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn over(&self) -> bool {
        self.state().over
    }

    /// Whether a poll found a far session in a turn a short while ago:
    /// within the plan's `worked`.
    fn working(&self) -> bool {
        let fresh = crate::hooks::now_ms() - self.plan.worked.as_millis() as i64;
        self.writer.working(fresh)
    }

    /// Waits `time`, or less once the machine is retired; whether it is.
    /// A wait longer than the quick pace looks once in that pace whether
    /// a session has taken a turn, and ends if one has: the quick pace
    /// begins with the turn, not at the long wait's end. A machine with
    /// no reader waits it out.
    fn pause(&self, time: Duration) -> bool {
        let began = Instant::now();
        loop {
            let left = time.saturating_sub(began.elapsed());
            let step = left.min(self.plan.live);
            let state = self.state();
            let (state, _) = (self.wake)
                .wait_timeout_while(state, step, |state| !state.over)
                .unwrap_or_else(PoisonError::into_inner);
            let (over, without) = (state.over, state.without.is_some());
            drop(state);
            if over || step == left || (!without && self.working()) {
                return over;
            }
        }
    }

    fn work(&self) {
        let mut wait = self.plan.first;
        while !self.pause(wait) {
            let ticked = self.tick();
            let without = self.state().without.is_some();
            wait = self.plan.wait(ticked, self.working(), without);
        }
    }

    /// The machine is retired, or client hooks are off: no read begins
    /// from now on, and what a read under way brings back is dropped by
    /// the writer, which has stopped. Nothing waits here: a read that is
    /// out ends by itself. Whatever the slot knew of a far reader is not
    /// known any more, so a machine that is added again probes first.
    pub(crate) fn stop(&self) {
        self.state().over = true;
        self.wake.notify_all();
        self.slots.retired(&self.key);
    }

    /// The machine's two ages, why it has no history where it has none,
    /// and how many of its sessions the mirror refused.
    pub(crate) fn ages(&self) -> Ages {
        let ages = {
            let state = self.state();
            Ages {
                refused: state.refused.values().sum(),
                ..Ages::of(state.ages.values(), state.without)
            }
        };
        let fresh = crate::hooks::now_ms() - FRESH.as_millis() as i64;
        let desktop = !self.writer.desktop_hosted(fresh).is_empty();
        Ages { desktop, ..ages }
    }

    /// One tick: every home of the machine in turn. Nothing is read
    /// while the machine has no link or its homes cannot be told.
    pub(crate) fn tick(&self) -> Ticked {
        let mut progress = (self.progress.lock()).unwrap_or_else(PoisonError::into_inner);
        let mut ticked = Ticked::default();
        let (Some(link), false) = (self.route.link(), self.over()) else {
            return ticked;
        };
        let Ok(inventory) = self.route.inventory() else {
            return ticked;
        };
        for home in inventory.homes {
            if self.over() {
                break;
            }
            let profile = home.profile().clone();
            let path = self.route.named(&home).into_owned();
            let at = progress.entry(profile.clone()).or_default();
            let ran = self.profile(&link, &path, &profile, at);
            (self.state().refused).insert(profile.clone(), at.refused());
            ticked.reads += ran.reads;
            let Some(failure) = ran.stopped else {
                ticked.overdue |= ran.overdue;
                self.state().without = None;
                continue;
            };
            // A home with no database has no sweep to be overdue.
            let none = tick::Stop::Read(Failure::Far(Far::NoDatabase));
            ticked.overdue |= ran.overdue && failure != none;
            let name = profile.name();
            self.log
                .line(&format!("history stopped profile={name}: {failure}"));
            match failure {
                tick::Stop::Read(Failure::Far(Far::NoDatabase)) => {
                    // Nothing to keep in step: it has no age either.
                    let mut state = self.state();
                    state.ages.remove(&profile);
                    state.refused.remove(&profile);
                }
                tick::Stop::Read(Failure::NoHistory) => {
                    self.state().without = Some(Without::Python);
                    break;
                }
                tick::Stop::Read(Failure::Refused(No::SqliteOld)) => {
                    self.state().without = Some(Without::SqliteOld);
                    break;
                }
                // Another reader is alive there, the link is gone, the
                // route changed or the machine is retired: the other
                // homes would fare the same. The next tick asks again.
                tick::Stop::Write
                | tick::Stop::Read(Failure::Busy | Failure::Link(_) | Failure::CutOff(_)) => break,
                // This home's database could not be read just now.
                tick::Stop::Read(_) => {}
            }
        }
        ticked
    }

    /// Whether a whole sweep of `reads` is overdue now.
    fn overdue(&self, reads: &Reads) -> bool {
        refresh::overdue(
            reads.sweep_done_ms,
            crate::hooks::now_ms(),
            self.plan.sweep_within,
        )
    }

    /// Makes the last whole sweep of `profile` that finished `by` older
    /// than it is, for the test of a sweep that is overdue.
    #[cfg(test)]
    pub(crate) fn backdate(&self, profile: &Profile, by: Duration) {
        let mut progress = (self.progress.lock()).unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = progress.get_mut(profile) {
            at.backdate(by.as_millis() as i64);
        }
    }

    #[cfg(test)]
    pub(crate) fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }

    /// Where the whole sweep of `profile` goes on from, and how many
    /// sessions it found changed and has yet to read.
    #[cfg(test)]
    pub(crate) fn sweep(&self, profile: &Profile) -> (i64, usize) {
        let progress = (self.progress.lock()).unwrap_or_else(PoisonError::into_inner);
        progress.get(profile).map_or((0, 0), tick::Progress::sweep)
    }
}
