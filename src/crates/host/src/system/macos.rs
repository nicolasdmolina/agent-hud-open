//! SMAppService uses only public frameworks. The Mac never posts system
//! notifications: the HUD island is its alert surface.
pub(crate) use super::unix::{client_command_line, configure_client_command, stop_client};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSBundle, NSError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}
fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundlePath().to_string().ends_with(".app") && bundle.bundleIdentifier().is_some()
}
fn service() -> Option<Retained<AnyObject>> {
    if !bundled() {
        return None;
    }
    let class = AnyClass::get(c"SMAppService")?;
    Some(unsafe { msg_send![class, mainAppService] })
}
/// `SMAppService.Status`. The main app says "not found" until it is first
/// registered, and registering from there works, so "not found" alone is no
/// fault: only a registration that fails, or leaves it so, is.
const NOT_REGISTERED: isize = 0;
const ENABLED: isize = 1;
const REQUIRES_APPROVAL: isize = 2;
const NOT_FOUND: isize = 3;
const NOT_FOUND_NOTICE: &str = "The login service was not found in this app bundle.";
/// Why the launch's registration failed, until the switch is used.
static STARTUP_FAILURE: Mutex<Option<String>> = Mutex::new(None);
/// The main app's login item: the OS's, or a test's.
trait MainApp {
    fn status(&self) -> isize;
    fn register(&self) -> Result<(), String>;
}
impl MainApp for AnyObject {
    fn status(&self) -> isize {
        unsafe { msg_send![self, status] }
    }
    fn register(&self) -> Result<(), String> {
        let mut error: *mut NSError = std::ptr::null_mut();
        let done: Bool = unsafe { msg_send![self, registerAndReturnError: &mut error] };
        done.as_bool().then_some(()).ok_or_else(|| reason(error))
    }
}
fn reason(error: *mut NSError) -> String {
    unsafe { error.as_ref() }.map_or_else(
        || "macOS gave no reason".to_owned(),
        |error| error.localizedDescription().to_string(),
    )
}
/// Registers when nothing is registered. Returns the warning Settings keeps
/// when that fails or the item is still not found.
fn register_at_startup<A: MainApp + ?Sized>(app: &A) -> Option<String> {
    if !matches!(app.status(), NOT_REGISTERED | NOT_FOUND) {
        return None;
    }
    match app.register() {
        Err(reason) => Some(format!("macOS could not register the login item: {reason}")),
        Ok(()) if app.status() == NOT_FOUND => Some(NOT_FOUND_NOTICE.to_owned()),
        Ok(()) => None,
    }
}
/// Settings' view of the raw status and any failed registration.
fn report(status: isize, failure: Option<&str>, name: &str) -> Value {
    let (state, notice) = match (status, failure) {
        (ENABLED, _) => ("enabled", None),
        (REQUIRES_APPROVAL, _) => (
            "requires-approval",
            Some(format!(
                "Approve {name} in System Settings → General → Login Items."
            )),
        ),
        (_, Some(failure)) => ("unavailable", Some(failure.to_owned())),
        (NOT_REGISTERED | NOT_FOUND, None) => ("disabled", None),
        _ => ("unavailable", None),
    };
    json!({"supported":true,"enabled":status==ENABLED,"status":state,"notice":notice})
}
fn startup_failure() -> std::sync::MutexGuard<'static, Option<String>> {
    STARTUP_FAILURE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
fn login() -> Value {
    let Some(service) = service() else {
        return json!({"supported":false,"enabled":false,"status":"unavailable","notice":"Launch at login requires a real app bundle on macOS 13 or later."});
    };
    let failure = startup_failure().clone();
    report(service.status(), failure.as_deref(), &app_name())
}
/// The name Login Items lists: this app bundle's own, so each build says its own.
fn app_name() -> String {
    let path = NSBundle::mainBundle().bundlePath().to_string();
    Path::new(&path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Agent HUD")
        .to_owned()
}
/// Returns whether the login item status may have changed.
pub(super) fn register_if_never_registered() -> bool {
    let Some(service) = service() else {
        return true;
    };
    let failure = register_at_startup(&*service);
    if let Some(failure) = &failure {
        eprintln!("Agent HUD launch at login: {failure}");
    }
    *startup_failure() = failure;
    true
}
pub(super) fn set_login(_identifier: &str, enabled: bool) -> Result<(), String> {
    let service = service().ok_or("Launch at login requires a real macOS app bundle")?;
    let current = service.status();
    if enabled && current == ENABLED || !enabled && matches!(current, NOT_REGISTERED | NOT_FOUND) {
        *startup_failure() = None;
        return Ok(());
    }
    let mut error: *mut NSError = std::ptr::null_mut();
    let success: Bool = unsafe {
        if enabled {
            msg_send![&*service,registerAndReturnError:&mut error]
        } else {
            msg_send![&*service,unregisterAndReturnError:&mut error]
        }
    };
    if success.as_bool() {
        *startup_failure() = None;
        Ok(())
    } else {
        Err(format!(
            "macOS could not update the login item ({}); check Login Items in System Settings.",
            reason(error)
        ))
    }
}
pub(super) fn status(_identifier: &str) -> Result<Value, String> {
    Ok(
        json!({"platform":"macos","launchAtLogin":login(),"sound":{"supported":true,"notice":null,"names":sound_names()}}),
    )
}
const SOUNDS: &str = "/System/Library/Sounds";
/// The system sounds Settings offers, sorted: each one `play_sound` can play
/// and Core's settings accept.
fn sound_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(SOUNDS)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "aiff").then_some(())?;
            let name = path.file_stem()?.to_str()?.to_owned();
            (super::sound::plain(&name) && name.trim() == name).then_some(name)
        })
        .collect();
    names.sort();
    names
}
/// Plays one of macOS's own alert sounds, from `/System/Library/Sounds`, and
/// waits until it ends. `afplay` is a process of its own, so the app never
/// waits on audio.
pub(super) fn play_sound(name: &str) -> Result<(), String> {
    let path = Path::new(SOUNDS).join(format!("{name}.aiff"));
    if !path.is_file() {
        return Err(format!("macOS has no system sound named {name}"));
    }
    let status = std::process::Command::new("/usr/bin/afplay")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("afplay could not start: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("afplay ended with {status}"))
    }
}

/// Settings looks only where `cli_candidates` says.
pub(crate) fn bundled_cli(_command: &str) -> Option<PathBuf> {
    None
}
pub(crate) fn cli_candidates(home: &Path, command: &str, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    cli_candidates_with(
        home,
        command,
        dirs,
        |path| {
            use std::os::unix::fs::PermissionsExt;
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        },
        agent_hud_runtime::registered_applications,
    )
}

fn cli_candidates_with(
    home: &Path,
    command: &str,
    dirs: Vec<PathBuf>,
    executable: impl Fn(&Path) -> bool,
    registered_applications: impl FnOnce(&str) -> Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates = vec![];
    if command == "codex" {
        // The app's own command line, where the engine looks for it too
        // (platform `engine_candidates`): Codex.app, or the ChatGPT app that
        // carries Codex inside it.
        let bundled = |app: PathBuf| {
            [
                "Contents/Resources/codex-cli/bin/codex",
                "Contents/Resources/codex",
            ]
            .map(|tail| app.join(tail))
        };
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for app in ["Codex.app", "ChatGPT.app"] {
                candidates.extend(bundled(root.join(app)));
            }
        }
        // Ask Launch Services only when the known bundles have no executable.
        if !candidates.iter().any(|path| executable(path)) {
            for app in registered_applications("com.openai.codex") {
                candidates.extend(bundled(app));
            }
        }
    }
    if matches!(command, "agy" | "antigravity") {
        candidates.push(home.join(".antigravity/antigravity/bin").join(command));
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for app in ["Antigravity.app", "Antigravity IDE.app"] {
                for tail in [
                    "Contents/Resources/app/bin/agy",
                    "Contents/Resources/app/bin/antigravity",
                    "Contents/Resources/bin/agy",
                ] {
                    candidates.push(root.join(app).join(tail));
                }
            }
        }
    }
    candidates.extend(dirs.into_iter().map(|dir| dir.join(command)));
    if command == "codex" {
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|path| seen.insert(path.clone()));
    }
    candidates
}

pub(crate) fn private_output(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A login item whose status is `before` until it is asked to register,
    /// then `after`.
    struct Fake {
        before: isize,
        after: isize,
        register: Result<(), String>,
        attempts: std::cell::Cell<usize>,
    }
    impl Fake {
        fn new(before: isize, register: Result<(), String>, after: isize) -> Self {
            let attempts = std::cell::Cell::new(0);
            Self {
                before,
                after,
                register,
                attempts,
            }
        }
    }
    impl MainApp for Fake {
        fn status(&self) -> isize {
            if self.attempts.get() == 0 {
                self.before
            } else {
                self.after
            }
        }
        fn register(&self) -> Result<(), String> {
            self.attempts.set(self.attempts.get() + 1);
            self.register.clone()
        }
    }

    #[test]
    fn a_failed_registration_from_not_found_keeps_its_warning() {
        let app = Fake::new(NOT_FOUND, Err("Operation not permitted".into()), NOT_FOUND);
        let failure = register_at_startup(&app);
        assert_eq!(app.attempts.get(), 1);
        let shown = report(app.status(), failure.as_deref(), "Agent HUD Fork");
        assert_eq!(shown["status"], "unavailable");
        assert_eq!(shown["enabled"], false);
        assert_eq!(
            shown["notice"],
            "macOS could not register the login item: Operation not permitted"
        );
        let still = Fake::new(NOT_FOUND, Ok(()), NOT_FOUND);
        let failure = register_at_startup(&still);
        assert_eq!(failure.as_deref(), Some(NOT_FOUND_NOTICE));
        assert_eq!(
            report(NOT_FOUND, failure.as_deref(), "x")["notice"],
            NOT_FOUND_NOTICE
        );
    }

    #[test]
    fn launch_registers_from_not_registered_or_not_found_only() {
        for before in [NOT_REGISTERED, NOT_FOUND] {
            let app = Fake::new(before, Ok(()), ENABLED);
            assert_eq!(register_at_startup(&app), None);
            assert_eq!(app.attempts.get(), 1);
            assert_eq!(report(app.status(), None, "x")["status"], "enabled");
        }
        for before in [ENABLED, REQUIRES_APPROVAL] {
            let app = Fake::new(before, Err("never asked".into()), before);
            assert_eq!(register_at_startup(&app), None);
            assert_eq!(app.attempts.get(), 0);
        }
    }

    #[test]
    fn not_found_without_a_failed_registration_is_plainly_off() {
        let shown = report(NOT_FOUND, None, "x");
        assert_eq!(shown["status"], "disabled");
        assert_eq!(shown["notice"], Value::Null);
        let waiting = report(REQUIRES_APPROVAL, Some("old failure"), "Agent HUD Fork");
        assert_eq!(waiting["status"], "requires-approval");
        assert_eq!(
            waiting["notice"],
            "Approve Agent HUD Fork in System Settings → General → Login Items."
        );
    }

    #[test]
    fn settings_offers_the_system_sounds_by_plain_name_in_order() {
        let names = sound_names();
        assert!(names.iter().any(|name| name == "Glass"));
        assert!(names.iter().any(|name| name == "Hero"));
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(names.iter().all(|name| super::super::sound::plain(name)));
        assert_eq!(status("").unwrap()["sound"]["names"], json!(names));
    }

    #[test]
    fn codex_is_found_in_the_chatgpt_app_too() {
        let home = Path::new("/Users/alice");
        let dir = PathBuf::from("/opt/homebrew/bin");
        let candidates =
            cli_candidates_with(home, "codex", vec![dir.clone()], |_| false, |_| vec![]);
        for path in [
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
            home.join("Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"),
        ] {
            assert!(candidates.contains(&path), "{path:?} is not a candidate");
        }
        assert_eq!(candidates.last(), Some(&dir.join("codex")));
        let agy_candidates = cli_candidates_with(
            home,
            "agy",
            vec![dir.clone()],
            |_| false,
            |_| panic!("only Codex asks Launch Services"),
        );
        for path in [
            home.join(".antigravity/antigravity/bin/agy"),
            PathBuf::from("/Applications/Antigravity.app/Contents/Resources/app/bin/agy"),
            home.join("Applications/Antigravity.app/Contents/Resources/app/bin/agy"),
        ] {
            assert!(
                agy_candidates.contains(&path),
                "{path:?} is not an agy candidate"
            );
        }
        assert_eq!(
            cli_candidates_with(
                home,
                "claude",
                vec![dir.clone()],
                |_| false,
                |_| panic!("only Codex asks Launch Services"),
            ),
            [dir.join("claude")]
        );
    }

    #[test]
    fn known_codex_executables_skip_registered_applications() {
        let home = Path::new("/Users/alice");
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for app in ["Codex.app", "ChatGPT.app"] {
                for tail in [
                    "Contents/Resources/codex-cli/bin/codex",
                    "Contents/Resources/codex",
                ] {
                    let known = root.join(app).join(tail);
                    let candidates = cli_candidates_with(
                        home,
                        "codex",
                        vec![],
                        |path| path == known.as_path(),
                        |_| panic!("a known executable needs no Launch Services lookup"),
                    );
                    assert_eq!(candidates.len(), 8);
                    assert!(candidates.contains(&known));
                }
            }
        }
    }

    #[test]
    fn registered_codex_apps_follow_known_bundles_before_path() {
        let mut queried = false;
        let candidates = cli_candidates_with(
            Path::new("/Users/alice"),
            "codex",
            vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/bin"),
            ],
            |_| false,
            |id| {
                assert_eq!(id, "com.openai.codex");
                queried = true;
                vec![
                    PathBuf::from("/Applications/Renamed.app"),
                    PathBuf::from("/Volumes/External/Codex.app"),
                ]
            },
        );
        assert!(queried);
        assert_eq!(
            candidates,
            [
                "/Applications/Codex.app/Contents/Resources/codex-cli/bin/codex",
                "/Applications/Codex.app/Contents/Resources/codex",
                "/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex",
                "/Applications/ChatGPT.app/Contents/Resources/codex",
                "/Users/alice/Applications/Codex.app/Contents/Resources/codex-cli/bin/codex",
                "/Users/alice/Applications/Codex.app/Contents/Resources/codex",
                "/Users/alice/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex",
                "/Users/alice/Applications/ChatGPT.app/Contents/Resources/codex",
                "/Applications/Renamed.app/Contents/Resources/codex-cli/bin/codex",
                "/Applications/Renamed.app/Contents/Resources/codex",
                "/Volumes/External/Codex.app/Contents/Resources/codex-cli/bin/codex",
                "/Volumes/External/Codex.app/Contents/Resources/codex",
                "/opt/homebrew/bin/codex",
                "/usr/bin/codex",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn codex_candidates_keep_the_first_of_repeated_paths() {
        let candidates = cli_candidates_with(
            Path::new("/Users/alice"),
            "codex",
            vec![
                PathBuf::from("/Applications/Renamed.app/Contents/Resources"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ],
            |_| false,
            |_| {
                vec![
                    PathBuf::from("/Applications/Renamed.app"),
                    PathBuf::from("/Applications/Codex.app"),
                    PathBuf::from("/Applications/Renamed.app"),
                ]
            },
        );
        assert_eq!(candidates.len(), 11);
        assert_eq!(
            &candidates[8..],
            [
                "/Applications/Renamed.app/Contents/Resources/codex-cli/bin/codex",
                "/Applications/Renamed.app/Contents/Resources/codex",
                "/opt/homebrew/bin/codex",
            ]
            .map(PathBuf::from)
        );
    }
}
