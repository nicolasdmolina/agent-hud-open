//! Agent HUD Fork (ticket 78, FORK.md "Inspector off"): upstream's inspector
//! stays off, by one switch, `INSPECTS`. Each of the inspector's entry points
//! asks it first and does nothing while it is off, so no caller can reach
//! around it: the browser note writer (`hosts::install`), the picture prune
//! (`content::prune`) and writer (`Picture::save`), the clipboard watch
//! (`watch::start`, `watch::summon`), the browser relay (`browser::listen`),
//! the shortcut (`shortcut::apply`), the window's command and its dispatch
//! (`commands`) and the markup save (`marking::save`). Settings, told by
//! `startup_mode`, hides the Experimental page while it is off. Nothing is
//! read from, written to or removed from any browser's folder. True gives
//! all of it back.

/// The fork's one switch for upstream's inspector.
const INSPECTS: bool = false;

/// Whether the inspector is on in this build.
pub(crate) fn on() -> bool {
    INSPECTS
}

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
