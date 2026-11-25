//! Agent HUD Fork (ticket 78, FORK.md): upstream's inspector stays off. No
//! clipboard watch, no browser-extension relay, no ⌥Z shortcut, and no note
//! in any browser's folder; a note an earlier fork build left there is taken
//! away, and one naming any other app is left alone. Settings hides the
//! Experimental page that holds the inspector's switches
//! (`app/src/settings/pages.ts`). To give the inspector back, set
//! `INSPECTS` to true and drop "experimental" from that page's filter.
use super::hosts;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The fork's switch for upstream's inspector.
const INSPECTS: bool = false;

/// What launch starts of the inspector.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Parts {
    /// The thread that reads the clipboard's change counter.
    pub(super) watch: bool,
    /// The socket the browser extension's relay speaks to.
    pub(super) relay: bool,
    /// The global shortcut that calls the clipboard window up.
    pub(super) shortcut: bool,
}

pub(super) fn parts(demo: bool) -> Parts {
    let on = INSPECTS && !demo;
    Parts {
        watch: on,
        relay: on,
        shortcut: on,
    }
}

/// The browsers' native-messaging notes, at launch: upstream writes one into
/// each browser folder; the fork removes only its own.
pub(super) fn browser_notes(home: &Path, executable: &Path) {
    if INSPECTS {
        hosts::install(home, executable);
    } else {
        remove_own(&hosts::folders(home), executable);
    }
}

fn remove_own(folders: &[PathBuf], executable: &Path) {
    for folder in folders {
        let file = folder.join(format!("{}.json", hosts::NAME));
        if names_this_app(&file, executable) {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// The note is upstream's, and its host is this app: this very executable,
/// or one of the same name (an earlier build of the fork, from elsewhere).
/// The official app's executable is named `Agent HUD`, the fork's
/// `Agent HUD Fork`.
fn names_this_app(file: &Path, executable: &Path) -> bool {
    let Some(note) = std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return false;
    };
    let Some(host) = note["path"].as_str().map(Path::new) else {
        return false;
    };
    note["name"] == hosts::NAME
        && (host == executable
            || host
                .file_name()
                .is_some_and(|name| Some(name) == executable.file_name()))
}

#[cfg(test)]
#[path = "fork_tests.rs"]
mod tests;
