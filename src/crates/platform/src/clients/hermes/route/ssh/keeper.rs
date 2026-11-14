//! Keeps the link to one far machine open. A thread of its own opens it,
//! watches it, and opens it again when it is lost, waiting longer each
//! time and never giving up: the Mac slept, the network dropped, the far
//! machine restarted. Nothing that follows a gateway waits on a sign-in:
//! whoever needs the link takes the one that is open now, or hears that
//! there is none.
//!
//! Each link that opens has a number. What was made on one link (a
//! forward, an admitted gateway) is gone with it, and is known by that
//! number not to belong to the next.
use crate::ssh_link::{Failure, Folder, Link, Lost, Settings, Target};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// The first wait before opening again, doubled each time it fails.
const FIRST: Duration = Duration::from_millis(250);
/// The longest wait before opening again.
const LIMIT: Duration = Duration::from_secs(30);
/// How often the master's process is looked at, which costs nothing.
const GLANCE: Duration = Duration::from_millis(500);
/// How often the master is asked whether it still answers.
const CHECK: Duration = Duration::from_secs(15);

/// What is said of the link as it comes and goes, for the log.
pub(crate) type Says = Arc<dyn Fn(&str) + Send + Sync>;

pub(super) struct Keeper {
    kept: Arc<Kept>,
}

struct Kept {
    folder: Folder,
    target: Target,
    settings: Settings,
    says: Says,
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    active: bool,
    link: Option<Arc<Link>>,
    /// Counts the links that opened.
    number: u64,
    /// Why there is no link, once one failed to open or was lost.
    down: Option<Failure>,
}

impl Keeper {
    pub(super) fn start(
        folder: Folder,
        target: Target,
        settings: Settings,
        says: Says,
    ) -> Result<Self, String> {
        let state = State {
            active: true,
            link: None,
            number: 0,
            down: None,
        };
        let kept = Arc::new(Kept {
            folder,
            target,
            settings,
            says,
            state: Mutex::new(state),
            changed: Condvar::new(),
        });
        let owned = kept.clone();
        thread::Builder::new()
            .name("Hermes far link".into())
            .spawn(move || owned.keep())
            .map_err(|_| "the link to a far Hermes could not be kept".to_owned())?;
        Ok(Self { kept })
    }

    /// The link that is open now, with its number.
    pub(super) fn link(&self) -> Option<(Arc<Link>, u64)> {
        let state = self.kept.state();
        Some((state.link.clone()?, state.number))
    }

    /// Why there is no link, where one failed to open or was lost.
    pub(super) fn down(&self) -> Option<Failure> {
        let state = self.kept.state();
        state.link.is_none().then_some(state.down).flatten()
    }

    /// Closes the link and opens no other. The master is asked to exit and
    /// confirmed gone before this returns; one that is still signing in is
    /// ended by the thread that started it, which gives up at once.
    pub(super) fn stop(&self) {
        let link = {
            let mut state = self.kept.state();
            state.active = false;
            state.link.take()
        };
        self.kept.changed.notify_all();
        if let Some(link) = link {
            link.close();
        }
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Kept {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn active(&self) -> bool {
        self.state().active
    }

    /// Waits `time`, or less once the keeper stops.
    fn pause(&self, time: Duration) {
        let state = self.state();
        let _ = self
            .changed
            .wait_timeout_while(state, time, |state| state.active);
    }

    fn keep(&self) {
        let mut wait = FIRST;
        while self.active() {
            let wanted = || self.active();
            match Link::open_while(&self.folder, &self.target, &self.settings, &wanted) {
                Ok(link) => {
                    let link = Arc::new(link);
                    if !self.publish(&link) {
                        link.close();
                        return;
                    }
                    wait = FIRST;
                    let lost = self.watch(&link);
                    self.withdraw(&link, lost);
                    link.close();
                }
                Err(failure) => self.failed(failure),
            }
            self.pause(wait);
            wait = (wait * 2).min(LIMIT);
        }
    }

    /// The link is the one to use from now on. False once stopped.
    fn publish(&self, link: &Arc<Link>) -> bool {
        let mut state = self.state();
        if !state.active {
            return false;
        }
        state.link = Some(link.clone());
        state.number += 1;
        state.down = None;
        let number = state.number;
        drop(state);
        (self.says)(&format!("link up number={number}"));
        self.changed.notify_all();
        true
    }

    /// Looks at the link until it is lost (why) or the keeper stops (None).
    fn watch(&self, link: &Link) -> Option<Lost> {
        let mut checked = Instant::now();
        loop {
            self.pause(GLANCE);
            if !self.active() {
                return None;
            }
            if let Err(lost) = link.running() {
                return Some(lost);
            }
            if checked.elapsed() >= CHECK {
                checked = Instant::now();
                if let Err(lost) = link.alive() {
                    return Some(lost);
                }
            }
        }
    }

    fn withdraw(&self, link: &Arc<Link>, lost: Option<Lost>) {
        let mut state = self.state();
        let same = |held: &Arc<Link>| Arc::ptr_eq(held, link);
        if state.link.as_ref().is_some_and(same) {
            state.link = None;
        }
        if let Some(lost) = lost {
            state.down = Some(Failure::Lost(lost));
            drop(state);
            (self.says)(&format!("link lost why={}", lost.reason()));
        }
        self.changed.notify_all();
    }

    /// No link opened. Said once for each reason, however often it fails.
    fn failed(&self, failure: Failure) {
        let mut state = self.state();
        if !state.active {
            return;
        }
        let news = state.down != Some(failure);
        state.down = Some(failure);
        drop(state);
        if news {
            (self.says)(&format!("link not open why={failure}"));
        }
    }
}
