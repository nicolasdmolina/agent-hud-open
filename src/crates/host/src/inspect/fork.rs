//! Agent HUD Fork (ticket 78, FORK.md "Inspector off"): upstream's inspector
//! stays off, by one switch, `INSPECTS`. Each of the inspector's entry points
//! asks it first and does nothing while it is off, so no caller can reach
//! around it: the browser note writer (`hosts::install`), the picture prune
//! (`content::prune`) and writer (`Picture::save`), the clipboard watch
//! (`watch::start`, `watch::summon`), the browser relay (`browser::listen`),
//! the shortcut (`shortcut::apply`), the window's command and its dispatch
//! (`commands`), the markup save (`marking::save`) and the browser's native
//! host (`browser::relay`'s exchange, which ends the process at once). Settings, told by
//! `startup_mode`, hides the Experimental page while it is off. Nothing is
//! read from, written to or removed from any browser's folder. True gives
//! all of it back.

/// The fork's one switch for upstream's inspector.
const INSPECTS: bool = false;

/// Whether the inspector is on in this build.
pub(crate) fn on() -> bool {
    INSPECTS
}

/// The whole guard of an entry point: returns at once, with the value given,
/// while the inspector is off. One statement, so it cannot be half removed.
macro_rules! off_return {
    () => {
        if !$crate::inspect::fork::on() {
            return;
        }
    };
    ($value:expr) => {
        if !$crate::inspect::fork::on() {
            return $value;
        }
    };
}
pub(super) use off_return;

/// For an entry point that answers: refused while the inspector is off.
pub(super) fn admit() -> Result<(), String> {
    if on() {
        Ok(())
    } else {
        Err("The inspector is off in this build".into())
    }
}

#[cfg(test)]
#[path = "fork_tests.rs"]
mod tests;
