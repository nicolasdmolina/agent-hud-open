//! Agent HUD Fork (ticket 78): with the inspector off, launch leaves the
//! browsers' folders and `~/.agenthud/tmp` alone and starts nothing, and only
//! a note that is provably the fork's is removed.
use super::*;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

const FORK: &str = "/Applications/Agent HUD Fork.app/Contents/MacOS/Agent HUD Fork";
const OFFICIAL: &str = "/Applications/Agent HUD.app/Contents/MacOS/Agent HUD";

/// Launch's steps, as asked for, in a fake home: the note and picture steps
/// do what the app's do; the others are only written down.
struct FakeHome<'a> {
    home: &'a Path,
    asked: Vec<&'static str>,
}

impl Launch for FakeHome<'_> {
    fn prune_pictures(&mut self) {
        self.asked.push("prune");
        super::super::content::prune(&self.home.join(".agenthud/tmp"), 7);
    }
    fn write_browser_notes(&mut self) {
        self.asked.push("write notes");
        hosts::install(self.home, Path::new(FORK));
    }
    fn remove_own_browser_notes(&mut self) {
        self.asked.push("remove own notes");
        remove_own(self.home, Path::new(FORK));
    }
    fn start_watch(&mut self) {
        self.asked.push("watch");
    }
    fn start_relay(&mut self) {
        self.asked.push("relay");
    }
    fn hold_shortcut(&mut self) {
        self.asked.push("shortcut");
    }
}

/// A fake home with Chromium browser folders in each system's layout, the
/// host-manifest folders the app looks in (none on Windows), and a picture
/// older than a week in `~/.agenthud/tmp`.
fn fake_home() -> (tempfile::TempDir, Vec<PathBuf>, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    for browser in [
        "Library/Application Support/Google/Chrome",
        "Library/Application Support/Arc/User Data",
        ".config/google-chrome",
        ".config/chromium",
    ] {
        fs::create_dir_all(home.path().join(browser)).unwrap();
    }
    let tmp = home.path().join(".agenthud/tmp");
    fs::create_dir_all(&tmp).unwrap();
    let picture = tmp.join("clip-old.png");
    fs::write(&picture, b"png").unwrap();
    let nine_days = SystemTime::now() - Duration::from_secs(9 * 86_400);
    fs::File::options()
        .write(true)
        .open(&picture)
        .unwrap()
        .set_modified(nine_days)
        .unwrap();
    let folders = hosts::folders(home.path());
    (home, folders, picture)
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

fn write_note(folder: &Path, host: &str) {
    fs::create_dir_all(folder).unwrap();
    let value = serde_json::json!({ "name": hosts::NAME, "path": host, "type": "stdio" });
    fs::write(note(folder), value.to_string()).unwrap();
}

#[test]
fn launch_with_the_switch_off_only_removes_its_own_notes() {
    let (home, folders, picture) = fake_home();
    let before = files_under(home.path());
    let mut steps = FakeHome {
        home: home.path(),
        asked: Vec::new(),
    };
    launch(false, &mut steps);
    assert_eq!(steps.asked, ["remove own notes"]);
    assert_eq!(files_under(home.path()), before);
    assert!(
        picture.is_file(),
        "a picture the fork never saved was pruned"
    );
    for folder in &folders {
        assert!(!folder.exists(), "{} was made", folder.display());
    }
    let mut demo = FakeHome {
        home: home.path(),
        asked: Vec::new(),
    };
    launch(true, &mut demo);
    assert!(demo.asked.is_empty());
}

#[test]
fn the_switch_on_gives_upstreams_launch_back() {
    let (home, folders, picture) = fake_home();
    let mut steps = FakeHome {
        home: home.path(),
        asked: Vec::new(),
    };
    launch_with(true, false, &mut steps);
    assert_eq!(
        steps.asked,
        ["prune", "write notes", "watch", "relay", "shortcut"]
    );
    assert!(!picture.exists());
    assert!(folders.iter().all(|folder| note(folder).is_file()));
    let mut demo = FakeHome {
        home: home.path(),
        asked: Vec::new(),
    };
    launch_with(true, true, &mut demo);
    assert_eq!(demo.asked, ["prune"]);
}

#[test]
fn the_inspector_is_off_and_its_command_refused() {
    assert!(!on());
    assert!(admit().is_err());
    assert!(admit_with(true).is_ok());
}

#[test]
fn a_note_naming_this_executable_is_removed_and_the_official_one_kept() {
    let (home, folders, _) = fake_home();
    let mut kept = Vec::new();
    for (index, folder) in folders.iter().enumerate() {
        let official = index % 2 == 1;
        write_note(folder, if official { OFFICIAL } else { FORK });
        if official {
            kept.push((note(folder), fs::read(note(folder)).unwrap()));
        }
    }
    remove_own(home.path(), Path::new(FORK));
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

/// An app bundle at `root/<name>.app` with an executable named
/// `Agent HUD Fork`, and its Info.plist as given (none for None).
fn bundle(root: &Path, name: &str, plist: Option<&str>) -> String {
    let contents = root.join(format!("{name}.app/Contents"));
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    if let Some(plist) = plist {
        fs::write(contents.join("Info.plist"), plist).unwrap();
    }
    let executable = contents.join("MacOS/Agent HUD Fork");
    fs::write(&executable, b"").unwrap();
    executable.to_string_lossy().into_owned()
}

fn xml_plist(identifier: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleExecutable</key>\n\t<string>Agent HUD Fork</string>\n\t<key>CFBundleIdentifier</key>\n\t<string>{identifier}</string>\n</dict>\n</plist>\n"
    )
}

#[test]
fn only_a_note_proven_to_be_the_forks_is_its_own() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("NativeMessagingHosts");
    let file = note(&folder);
    let running = Path::new(FORK);
    let owned = |host: &str| {
        write_note(&folder, host);
        names_this_fork(&file, running)
    };
    // This very executable.
    assert!(owned(FORK));
    // A fork bundle elsewhere, by its Info.plist.
    assert!(owned(&bundle(
        root.path(),
        "Old Fork",
        Some(&xml_plist(FORK_BUNDLE_ID))
    )));
    // Named like the fork, but another app's bundle, or no bundle at all.
    assert!(!owned(&bundle(
        root.path(),
        "Other",
        Some(&xml_plist("com.example.other"))
    )));
    assert!(!owned(&bundle(
        root.path(),
        "Official",
        Some(&xml_plist("app.agenthud.mac"))
    )));
    assert!(!owned("/tmp/elsewhere/Agent HUD Fork"));
    // Unclear: no Info.plist, or one that is not XML.
    assert!(!owned(&bundle(root.path(), "No Plist", None)));
    assert!(!owned(&bundle(
        root.path(),
        "Binary",
        Some("bplist00\u{1}\u{2}")
    )));
    assert!(!owned(OFFICIAL));
    // Not upstream's note, or not a note.
    fs::write(
        &file,
        serde_json::json!({ "name": "other.host", "path": FORK }).to_string(),
    )
    .unwrap();
    assert!(!names_this_fork(&file, running));
    fs::write(
        &file,
        serde_json::json!({ "name": hosts::NAME }).to_string(),
    )
    .unwrap();
    assert!(!names_this_fork(&file, running));
    fs::write(&file, "not json").unwrap();
    assert!(!names_this_fork(&file, running));
    assert!(!names_this_fork(&folder.join("missing.json"), running));
}

#[test]
fn what_is_not_a_plain_file_stays() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("NativeMessagingHosts");
    fs::create_dir_all(note(&folder)).unwrap();
    assert!(!names_this_fork(&note(&folder), Path::new(FORK)));
}

#[test]
fn the_bundle_identifier_is_the_forks_overlay() {
    let overlay: Value = serde_json::from_str(include_str!(
        "../../../../apps/desktop/src-tauri/tauri.fork.conf.json"
    ))
    .unwrap();
    assert_eq!(overlay["identifier"], FORK_BUNDLE_ID);
}
