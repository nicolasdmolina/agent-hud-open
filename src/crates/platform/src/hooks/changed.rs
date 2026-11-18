//! Files a native channel wrote itself, for core to read. A channel that
//! keeps what it learns in a file core reads as a log (a far machine's
//! mirror, `clients/hermes/mirror.rs`) says so after each write
//! (`Changed`): the paths wait here, and the channel's wake asks for a
//! pass. That pass hands them to core (`AgentHudCore::written_files`),
//! which takes them for changed as it takes the file watcher's, and
//! reads them at once (`agent_hud_runtime`, `request`), also one whose
//! last read failed and that would be left be for 30 s otherwise.
//!
//! The file watcher reports the same writes when it can, and that only
//! makes the read sooner. Nothing may rest on it: its events arrive late
//! for a file held open, a folder made after launch is not watched yet,
//! and the stat check between passes looks only at logs core has read
//! before.
use super::{Hooks, Wake};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

/// A native channel's word that these files of its own changed. It names
/// no client. It never reads a file and never waits on core: it may be
/// called from the thread that wrote.
pub type Changed = Arc<dyn Fn(&[PathBuf]) + Send + Sync>;

/// The paths told and not yet handed to core.
#[derive(Clone, Default)]
pub(super) struct Written(Arc<Mutex<HashSet<PathBuf>>>);

impl Written {
    /// The callback a channel is given: it keeps the paths, then wakes
    /// the collector with `wake`.
    pub(super) fn teller(&self, wake: Wake) -> Changed {
        let kept = self.0.clone();
        Arc::new(move |paths: &[PathBuf]| {
            if paths.is_empty() {
                return;
            }
            let mut written = kept.lock().unwrap_or_else(PoisonError::into_inner);
            written.extend(paths.iter().cloned());
            drop(written);
            wake();
        })
    }

    /// Every path told since the last call.
    pub(super) fn drain(&self) -> HashSet<PathBuf> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Hooks {
    /// What wakes the collector for a request from now on.
    pub fn set_waker(&self, wake: Wake) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = wake;
    }

    /// The files native channels said they wrote, since the last call.
    pub fn drain_file_changes(&self) -> HashSet<PathBuf> {
        self.written.drain()
    }

    /// The callback native channels are given, for the tests that tell
    /// what a channel would.
    pub fn files_changed(&self) -> Changed {
        self.written.teller(self.wake.clone())
    }
}
