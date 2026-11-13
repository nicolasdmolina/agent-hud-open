//! Agent HUD's data folder, named after the app's identifier on each system.
//! Every edition of the app uses the same one.
//!
//! Builds before the app took those identifiers kept their data in
//! `app.agenthud.next`. The app moves that folder once at launch (`adopt`),
//! while it holds the lock every copy that owns client hooks takes. Until
//! then `resolve` keeps naming the earlier folder, so no copy reads half of
//! each.
//!
//! The fork does neither (`ADOPTS_EARLIER`): `app.agenthud.next` belongs to
//! the official app, so the fork never uses, moves or merges it (FORK.md).
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// The folder's name on each system: the app's identifier there, as the
/// desktop app's Tauri configuration gives it. A build uses its own system's
/// name; the test below checks all three on every system. The fork's
/// identifier (`tauri.fork.conf.json`) on all three, so it never shares the
/// official app's folder (FORK.md).
#[allow(dead_code)]
pub(crate) const MAC: &str = "com.nickymolina.agenthud";
#[allow(dead_code)]
pub(crate) const WINDOWS: &str = MAC;
#[allow(dead_code)]
pub(crate) const LINUX: &str = MAC;
/// The name earlier builds used on every system.
const EARLIER: &str = "app.agenthud.next";
/// Whether this build uses and moves the earlier folder. Off in the fork.
const ADOPTS_EARLIER: bool = false;
/// The file every collector locks in the folder it uses. Once it is in this
/// build's folder, that folder is the one in use.
pub(crate) const LOCK: &str = "instance.lock";

/// The folder in use under `parent`: this build's once a collector has used
/// it, else an earlier build's while that one is still there.
pub(crate) fn resolve(parent: &Path, name: &str) -> PathBuf {
    let current = parent.join(name);
    let earlier = parent.join(EARLIER);
    if !ADOPTS_EARLIER || current.join(LOCK).exists() || !earlier.is_dir() {
        current
    } else {
        earlier
    }
}

/// An earlier build's folder under `parent`, while it is there.
pub(crate) fn earlier(parent: &Path) -> Option<PathBuf> {
    Some(parent.join(EARLIER)).filter(|path| ADOPTS_EARLIER && path.is_dir())
}

/// Moves an earlier build's folder under `parent` to `name`.
///
/// What is already at `name` before the move is usually the web view's data
/// (Windows keeps it in the folder named after the app). It joins the earlier
/// folder, which then moves whole, so the data never sits in two places. A
/// folder an earlier build wrote after the move adds only what this build's
/// folder lacks, and is removed once nothing of it is left.
pub(crate) fn adopt(parent: &Path, name: &str) -> io::Result<()> {
    let earlier = parent.join(EARLIER);
    if !ADOPTS_EARLIER || !earlier.is_dir() {
        return Ok(());
    }
    let current = parent.join(name);
    if current.join(LOCK).exists() {
        return merge(&earlier, &current);
    }
    if current.is_dir() {
        merge(&current, &earlier)?;
    }
    fs::rename(&earlier, &current)
}

/// Moves each entry of `from` that `into` lacks, and the entries inside the
/// folders both have. A file both have stays as `into` has it. `from` is
/// removed when that leaves it empty.
fn merge(from: &Path, into: &Path) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = into.join(entry.file_name());
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::rename(entry.path(), &target)?
            }
            Ok(existing) if existing.is_dir() && entry.file_type()?.is_dir() => {
                merge(&entry.path(), &target)?
            }
            _ => {}
        }
    }
    // Only an empty folder goes; one with a file `into` kept stays.
    let _ = fs::remove_dir(from);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn each_name_is_the_apps_identifier_on_its_system() {
        // The fork's overlay sets one identifier on every system (FORK.md).
        let tauri = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src-tauri");
        let file = "tauri.fork.conf.json";
        let config: serde_json::Value = serde_json::from_str(&read(&tauri.join(file))).unwrap();
        for name in [MAC, WINDOWS, LINUX] {
            assert_eq!(config["identifier"], name, "{file}");
        }
    }

    // The fork replaces upstream's migration tests (the earlier folder in
    // use until this one is, then moved or merged): it never adopts.
    #[test]
    fn the_earlier_folder_is_never_used_or_moved() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("data");
        let earlier = parent.join(EARLIER);
        write(&earlier.join("ledger.sqlite"), "official-canary");
        write(&earlier.join(LOCK), "");
        write(&earlier.join("hooks/inbox/event.json"), "event");
        for name in [MAC, WINDOWS, LINUX] {
            assert_eq!(resolve(&parent, name), parent.join(name));
            assert_eq!(super::earlier(&parent), None);
            adopt(&parent, name).unwrap();
            assert!(!parent.join(name).exists());
        }
        // A folder this build already uses does not pull the earlier one in.
        write(&parent.join(MAC).join(LOCK), "");
        adopt(&parent, MAC).unwrap();
        assert_eq!(read(&earlier.join("ledger.sqlite")), "official-canary");
        assert_eq!(read(&earlier.join("hooks/inbox/event.json")), "event");
        assert!(earlier.join(LOCK).exists());
        assert!(!parent.join(MAC).join("ledger.sqlite").exists());
    }
}
