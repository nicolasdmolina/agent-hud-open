//! How a follower asks for a look at every gateway of its machine, and
//! how the look waits to be asked (`discovery.rs`).
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex, PoisonError,
    },
    time::Duration,
};

/// Asks for a look at every gateway of the machine, however many ask.
#[derive(Default)]
pub(super) struct Signal {
    wanted: AtomicBool,
    waiting: (Mutex<()>, Condvar),
    /// The first look is over, whatever it found.
    looked: AtomicBool,
}

impl Signal {
    pub(super) fn want(&self) {
        self.wanted.store(true, Ordering::Release);
        self.wake();
    }

    /// Ends a wait that is under way, wanted or not.
    pub(super) fn wake(&self) {
        let (lock, waiting) = &self.waiting;
        let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.notify_all();
    }

    /// Whether the first look at the machine is over. Until it is, no
    /// follower polls: a profile two gateways claim must be known as
    /// that before either opens a card for it.
    pub(super) fn looked(&self) -> bool {
        self.looked.load(Ordering::Acquire)
    }

    pub(super) fn set_looked(&self) {
        self.looked.store(true, Ordering::Release);
    }

    pub(super) fn take(&self) -> bool {
        self.wanted.swap(false, Ordering::AcqRel)
    }

    /// Waits `time`, whoever asks for a look meanwhile, or less where
    /// `over` says there is nothing to wait for.
    pub(super) fn rest(&self, time: Duration, over: impl Fn() -> bool) {
        let (lock, waiting) = &self.waiting;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = waiting.wait_timeout_while(guard, time, |_| !over());
    }

    /// Waits `time`, for ever where there is none, or less where a look
    /// is wanted or `over` says there is nothing to wait for.
    pub(super) fn wait(&self, time: Option<Duration>, over: impl Fn() -> bool) {
        let (lock, waiting) = &self.waiting;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let idle = |_: &mut ()| !over() && !self.wanted.load(Ordering::Acquire);
        match time {
            Some(time) => drop(waiting.wait_timeout_while(guard, time, idle)),
            None => drop(waiting.wait_while(guard, idle)),
        }
    }
}
