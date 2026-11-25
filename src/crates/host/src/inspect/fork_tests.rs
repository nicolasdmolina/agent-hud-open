//! Agent HUD Fork (ticket 78): with the inspector off, each of its entry
//! points does nothing. The ones that need no running app are called here
//! directly in a fake home; the others are checked to open with the switch.
use super::super::{content, hosts};
use super::*;
use base64::Engine;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const FORK: &str = "/Applications/Agent HUD Fork.app/Contents/MacOS/Agent HUD Fork";
/// A 1×1 PNG, readable as a picture.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// Every entry under `root`, links not followed: a file's bytes, a link's
/// target, a folder as such.
fn inventory(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        let kind = fs::symlink_metadata(&path).unwrap().file_type();
        if kind.is_symlink() {
            found.insert(
                path.clone(),
                format!("link {:?}", fs::read_link(&path).unwrap()),
            );
        } else if kind.is_dir() {
            found.insert(path.clone(), "folder".into());
            found.extend(inventory(&path));
        } else {
            found.insert(path.clone(), format!("{:?}", fs::read(&path).unwrap()));
        }
    }
    found
}

fn note(host: &str) -> String {
    serde_json::json!({ "name": "app.agenthud.inspect", "path": host, "type": "stdio" }).to_string()
}

#[test]
fn the_inspector_is_off_and_answers_nothing() {
    assert!(!on());
    assert!(admit().is_err());
}

#[test]
fn no_browser_folder_is_touched() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    let mac = root.join("Library/Application Support");
    let linux = root.join(".config");
    let elsewhere = root.join("elsewhere");
    let notes = [
        // A note naming the fork, one naming the official app, a malformed
        // one, and one linked to a file outside the browser's folder.
        (
            mac.join("Google/Chrome/NativeMessagingHosts"),
            Some(note(FORK)),
        ),
        (
            mac.join("Arc/User Data/NativeMessagingHosts"),
            Some(note("/Applications/Agent HUD.app/Contents/MacOS/Agent HUD")),
        ),
        (
            linux.join("google-chrome/NativeMessagingHosts"),
            Some(note(FORK)),
        ),
        (
            linux.join("chromium/NativeMessagingHosts"),
            Some("{ not json".into()),
        ),
        (mac.join("Vivaldi/NativeMessagingHosts"), None),
    ];
    for (folder, text) in &notes {
        fs::create_dir_all(folder).unwrap();
        if let Some(text) = text {
            fs::write(folder.join("app.agenthud.inspect.json"), text).unwrap();
        }
    }
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("target.json"), note(FORK)).unwrap();
    // Where the system lets a test make a link.
    #[allow(deprecated)]
    let _ = fs::soft_link(
        elsewhere.join("target.json"),
        mac.join("Vivaldi/NativeMessagingHosts/app.agenthud.inspect.json"),
    );
    // Browsers with no host-manifest folder yet.
    fs::create_dir_all(mac.join("BraveSoftware/Brave-Browser")).unwrap();
    fs::create_dir_all(linux.join("vivaldi")).unwrap();
    let before = inventory(root);
    hosts::install(root, Path::new(FORK));
    assert_eq!(inventory(root), before);
}

#[test]
fn no_picture_is_pruned() {
    let folder = tempfile::tempdir().unwrap();
    let picture = folder.path().join("clip-old.png");
    fs::write(&picture, b"png").unwrap();
    let nine_days = SystemTime::now() - Duration::from_secs(9 * 86_400);
    let file = fs::File::options().write(true).open(&picture).unwrap();
    file.set_modified(nine_days).unwrap();
    drop(file);
    content::prune(folder.path(), 7);
    assert!(
        picture.is_file(),
        "a picture the fork never saved was pruned"
    );
    assert_eq!(
        fs::metadata(&picture).unwrap().modified().unwrap(),
        nine_days
    );
}

#[test]
fn no_picture_is_saved() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join(".agenthud/tmp");
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    assert!(content::Picture::save(png, &folder, "test").is_none());
    assert!(!folder.exists());
}

/// The first statement in the body of `signature` in `source`.
fn first_statement<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} is gone"));
    let body = &source[start..];
    let open = body.find("{\n").unwrap() + 2;
    body[open..]
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//"))
        .unwrap()
}

/// The entry points that need a running app open with the switch, so no
/// caller reaches around it.
#[test]
fn every_entry_point_that_needs_the_app_asks_the_switch_first() {
    let refuse = "super::fork::admit()?;";
    let skip = "if !super::fork::on() {";
    for (file, source, signature, gate) in [
        ("watch.rs", include_str!("watch.rs"), "fn start(", skip),
        ("watch.rs", include_str!("watch.rs"), "fn summon(", skip),
        ("browser.rs", include_str!("browser.rs"), "fn listen(", skip),
        (
            "shortcut.rs",
            include_str!("shortcut.rs"),
            "fn apply(",
            skip,
        ),
        (
            "commands.rs",
            include_str!("commands.rs"),
            "fn inspect_request(",
            refuse,
        ),
        (
            "commands.rs",
            include_str!("commands.rs"),
            "fn answer(",
            refuse,
        ),
        ("marking.rs", include_str!("marking.rs"), "fn save(", refuse),
        ("hosts.rs", include_str!("hosts.rs"), "fn install(", skip),
        ("content.rs", include_str!("content.rs"), "fn prune(", skip),
        ("content.rs", include_str!("content.rs"), "fn save(", skip),
    ] {
        assert_eq!(
            first_statement(source, signature),
            gate,
            "{file} {signature}"
        );
    }
}
