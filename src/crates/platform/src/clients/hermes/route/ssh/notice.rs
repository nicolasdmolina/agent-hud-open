//! What a route says to whoever follows its machine: that the machine is
//! not covered any more and why, or that something raises a doubt worth a
//! fresh look at the whole machine. And the gate that keeps the commands
//! run over one link few at a time.
use std::{
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

/// What a wait on the notice ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Noticed {
    /// The machine is not covered, for this reason.
    Uncovered(String),
    /// A check failed for one home: the machine is to be looked at again.
    Doubt,
    /// Whoever waited wants no more.
    Over,
}

#[derive(Default)]
pub(super) struct Notice {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    uncovered: Option<String>,
    doubt: bool,
}

impl Notice {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The machine is not covered. The first reason is the one kept.
    pub(super) fn uncover(&self, why: &str) {
        self.state().uncovered.get_or_insert_with(|| why.to_owned());
        self.changed.notify_all();
    }

    pub(super) fn doubt(&self) {
        self.state().doubt = true;
        self.changed.notify_all();
    }

    pub(super) fn wake(&self) {
        // Held while waking, so a wait that just looked hears this.
        let _held = self.state();
        self.changed.notify_all();
    }

    /// Forgets what was said, when the machine is verified anew.
    pub(super) fn clear(&self) {
        *self.state() = State::default();
    }

    /// Waits until something is said, or `over` says to wait no more.
    pub(super) fn wait(&self, over: &dyn Fn() -> bool) -> Noticed {
        let mut state = self.state();
        loop {
            if over() {
                return Noticed::Over;
            }
            if let Some(why) = &state.uncovered {
                return Noticed::Uncovered(why.clone());
            }
            if std::mem::take(&mut state.doubt) {
                return Noticed::Doubt;
            }
            // By the clock too, since `over` changes without a word here.
            let (next, _) = self
                .changed
                .wait_timeout(state, Duration::from_millis(200))
                .unwrap_or_else(PoisonError::into_inner);
            state = next;
        }
    }
}

/// At most this many commands run over one link at a time: an `sshd`
/// takes ten sessions for one connection unless it is told otherwise, and
/// one too many is refused, which would read as a relay that was cut off.
const AT_ONCE: usize = 4;

#[derive(Default)]
pub(super) struct Gate {
    inside: Mutex<usize>,
    left: Condvar,
}

pub(super) struct Inside<'a>(&'a Gate);

impl Gate {
    pub(super) fn enter(&self) -> Inside<'_> {
        let inside = self.inside.lock().unwrap_or_else(PoisonError::into_inner);
        let mut inside = self
            .left
            .wait_while(inside, |inside| *inside >= AT_ONCE)
            .unwrap_or_else(PoisonError::into_inner);
        *inside += 1;
        Inside(self)
    }
}

impl Drop for Inside<'_> {
    fn drop(&mut self) {
        let mut inside = (self.0.inside.lock()).unwrap_or_else(PoisonError::into_inner);
        *inside -= 1;
        drop(inside);
        self.0.left.notify_one();
    }
}
