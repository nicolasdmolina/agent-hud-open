//! Agent HUD Fork (ticket 78): the inspector's launch leaves the browsers'
//! folders alone and starts nothing.
use super::*;
use std::fs;

const FORK: &str = "/Applications/Agent HUD Fork.app/Contents/MacOS/Agent HUD Fork";
const OFFICIAL: &str = "/Applications/Agent HUD.app/Contents/MacOS/Agent HUD";

/// A fake home with a Chromium browser's folder in each system's layout,
/// and the host-manifest folders the app would look in. Empty only where the
/// app writes no note at all (Windows).
fn home_with_browsers() -> (tempfile::TempDir, Vec<PathBuf>) {
    let home = tempfile::tempdir().unwrap();
    for browser in [
        "Library/Application Support/Google/Chrome",
        "Library/Application Support/Arc/User Data",
        ".config/google-chrome",
        ".config/chromium",
    ] {
        fs::create_dir_all(home.path().join(browser)).unwrap();
    }
    let folders = hosts::folders(home.path());
    (home, folders)
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn note(folder: &Path) -> PathBuf {
    folder.join(format!("{}.json", hosts::NAME))
}

#[test]
fn launch_writes_no_note_into_any_browser_folder() {
    let (home, folders) = home_with_browsers();
    let before = files_under(home.path());
    browser_notes(home.path(), Path::new(FORK));
    assert_eq!(files_under(home.path()), before);
    for folder in &folders {
        assert!(!folder.exists(), "{} was made", folder.display());
    }
    // Upstream's step, in the same home, writes one note per folder: the
    // check above looked where a note would go.
    hosts::install(home.path(), Path::new(FORK));
    assert!(folders.iter().all(|folder| note(folder).is_file()));
}

#[test]
fn a_note_an_earlier_fork_build_left_is_removed_and_the_official_one_kept() {
    let (home, folders) = home_with_browsers();
    // Upstream's own step leaves the fork's note in every folder; then every
    // other folder gets the official app's note in its place.
    hosts::install(home.path(), Path::new(FORK));
    let mut kept = Vec::new();
    for folder in folders.iter().skip(1).step_by(2) {
        let official =
            serde_json::json!({ "name": hosts::NAME, "path": OFFICIAL, "type": "stdio" });
        fs::write(note(folder), official.to_string()).unwrap();
        kept.push((note(folder), fs::read(note(folder)).unwrap()));
    }
    assert!(folders.iter().all(|folder| note(folder).is_file()));
    browser_notes(home.path(), Path::new(FORK));
    for (index, folder) in folders.iter().enumerate() {
        assert_eq!(
            note(folder).exists(),
            index % 2 == 1,
            "{}",
            folder.display()
        );
    }
    for (file, bytes) in kept {
        assert_eq!(fs::read(file).unwrap(), bytes);
    }
}

#[test]
fn only_upstreams_note_naming_this_app_is_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let file = note(folder.path());
    let fork = Path::new(FORK);
    let write = |value: serde_json::Value| fs::write(&file, value.to_string()).unwrap();
    write(serde_json::json!({ "name": hosts::NAME, "path": FORK }));
    assert!(names_this_app(&file, fork));
    // The same executable name, from another place: an earlier fork build.
    write(serde_json::json!({ "name": hosts::NAME, "path": "/tmp/x/Agent HUD Fork" }));
    assert!(names_this_app(&file, fork));
    write(serde_json::json!({ "name": hosts::NAME, "path": OFFICIAL }));
    assert!(!names_this_app(&file, fork));
    write(serde_json::json!({ "name": "other.host", "path": FORK }));
    assert!(!names_this_app(&file, fork));
    write(serde_json::json!({ "name": hosts::NAME }));
    assert!(!names_this_app(&file, fork));
    fs::write(&file, "not json").unwrap();
    assert!(!names_this_app(&file, fork));
}

#[test]
fn launch_starts_no_clipboard_watch_relay_or_shortcut() {
    let off = Parts {
        watch: false,
        relay: false,
        shortcut: false,
    };
    assert_eq!(parts(false), off);
    assert_eq!(parts(true), off);
}
