//! Agent HUD Fork (ticket 78, FORK.md "Inspector off"): upstream's inspector
//! stays off, by one switch, `INSPECTS`. Off, launch takes one step only: a
//! browser note an earlier fork build left is removed. No picture is pruned,
//! no note written, no clipboard watch, relay or shortcut started; the
//! window's command refuses every request; and Settings, told by
//! `startup_mode`, hides the Experimental page. True gives all of it back.
use super::hosts;
use serde_json::Value;
use std::path::Path;

/// The fork's one switch for upstream's inspector.
const INSPECTS: bool = false;

/// The fork's bundle identifier, as `tauri.fork.conf.json` sets it. A note
/// whose host sits in a bundle that says so is the fork's.
const FORK_BUNDLE_ID: &str = "com.nickymolina.agenthud";

/// Whether the inspector is on in this build.
pub(crate) fn on() -> bool {
    INSPECTS
}

/// The inspector's command, refused while it is off.
pub(super) fn admit() -> Result<(), String> {
    admit_with(INSPECTS)
}

fn admit_with(on: bool) -> Result<(), String> {
    if on {
        Ok(())
    } else {
        Err("The inspector is off in this build".into())
    }
}

/// Each thing launch can do of the inspector. `install` acts only through
/// these, in the order `launch` gives.
pub(super) trait Launch {
    /// Removes week-old pictures from `~/.agenthud/tmp`.
    fn prune_pictures(&mut self);
    /// Writes upstream's note into every browser folder.
    fn write_browser_notes(&mut self);
    /// Removes only a note that is provably the fork's.
    fn remove_own_browser_notes(&mut self);
    /// Starts the thread that reads the clipboard's change counter.
    fn start_watch(&mut self);
    /// Opens the socket the browser extension's relay speaks to.
    fn start_relay(&mut self);
    /// Holds the global shortcut that calls the clipboard window up.
    fn hold_shortcut(&mut self);
}

pub(super) fn launch(demo: bool, steps: &mut impl Launch) {
    launch_with(INSPECTS, demo, steps);
}

fn launch_with(on: bool, demo: bool, steps: &mut impl Launch) {
    if !on {
        if !demo {
            steps.remove_own_browser_notes();
        }
        return;
    }
    // Upstream's launch, as it was.
    steps.prune_pictures();
    if demo {
        return;
    }
    steps.write_browser_notes();
    steps.start_watch();
    steps.start_relay();
    steps.hold_shortcut();
}

/// Removes, from each browser folder, a note that is provably the fork's.
pub(super) fn remove_own(home: &Path, executable: &Path) {
    for folder in hosts::folders(home) {
        let file = folder.join(format!("{}.json", hosts::NAME));
        if names_this_fork(&file, executable) {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// The note is a plain file of upstream's, and its host is the fork: this
/// very executable, or one inside an app bundle whose Info.plist names the
/// fork's bundle identifier. Anything unreadable, linked or unclear stays.
fn names_this_fork(file: &Path, executable: &Path) -> bool {
    if !std::fs::symlink_metadata(file).is_ok_and(|meta| meta.is_file()) {
        return false;
    }
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
        && (host == executable || bundle_identifier(host).as_deref() == Some(FORK_BUNDLE_ID))
}

/// The bundle identifier of `<name>.app/Contents/MacOS/<executable>`, read
/// from the bundle's XML Info.plist; nothing is run. None for any other
/// shape, or a plist that cannot be read as XML.
fn bundle_identifier(executable: &Path) -> Option<String> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || bundle.extension()? != "app"
    {
        return None;
    }
    let plist = std::fs::read_to_string(contents.join("Info.plist")).ok()?;
    plist_string(&plist, "CFBundleIdentifier")
}

/// The string value of a top-level `key` in an XML plist.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after = plist
        .split_once(&format!("<key>{key}</key>"))?
        .1
        .trim_start();
    let value = after.strip_prefix("<string>")?.split_once("</string>")?.0;
    Some(value.trim().to_owned())
}

#[cfg(test)]
#[path = "fork_tests.rs"]
mod tests;
