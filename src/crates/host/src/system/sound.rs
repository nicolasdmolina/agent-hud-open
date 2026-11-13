//! Sound alerts. Core decides which moments deserve a sound (`soundAlerts` in
//! the snapshot); the host plays each alert id once, as a named system sound,
//! through the host file's `play_sound`. Each play is one line in `sound.log`
//! in the app's data folder, so a play can be checked without listening. The
//! line names the kind, the sound and the alert id, never session text.
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};
use tauri::Manager;

/// The log starts over past this size, so it never grows without bound.
const LOG_LIMIT: u64 = 256 * 1024;

pub struct Sounds {
    /// The alert ids the last snapshot carried. Core keeps an alert in its
    /// list until it is older than the list, and never brings one back, so
    /// an id seen once is never played again.
    played: Mutex<HashSet<String>>,
    log: PathBuf,
}

#[derive(Debug, PartialEq)]
pub(super) struct Alert {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) sound: String,
}

impl Sounds {
    /// `directory` is the data folder this run's Core uses; the demo has its own.
    pub fn new(directory: &Path) -> Self {
        Self {
            played: Mutex::new(HashSet::new()),
            log: directory.join("sound.log"),
        }
    }

    /// Plays, each on a thread of its own, the alerts this snapshot carries
    /// that the one before did not. Nothing plays while the setting is off.
    pub fn observe(&self, snapshot: &Value) {
        let alerts = fresh(
            &mut self.played.lock().unwrap_or_else(PoisonError::into_inner),
            snapshot,
        );
        for alert in alerts {
            let log = self.log.clone();
            std::thread::spawn(move || {
                let _ = play(&log, &alert.kind, &alert.sound, Some(&alert.id));
            });
        }
    }

    /// Plays one sound now, whatever the setting says: Settings' preview.
    /// Waits until it has played, so call it off the HUD's threads.
    pub fn preview(&self, name: &str) -> Result<(), String> {
        play(&self.log, "preview", name, None)
    }
}

/// The alerts in `snapshot` that are not in `played`, which becomes the ids
/// the snapshot carries. A snapshot without the list changes nothing.
pub(super) fn fresh(played: &mut HashSet<String>, snapshot: &Value) -> Vec<Alert> {
    let Some(items) = snapshot["soundAlerts"]["items"].as_array() else {
        return Vec::new();
    };
    let mut alerts = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        if seen.insert(id.to_owned()) && !played.contains(id) {
            alerts.push(Alert {
                id: id.to_owned(),
                kind: item["kind"].as_str().unwrap_or_default().to_owned(),
                sound: item["sound"].as_str().unwrap_or_default().to_owned(),
            });
        }
    }
    *played = seen;
    if snapshot["settings"]["sound"] != Value::Bool(true) {
        alerts.clear();
    }
    alerts
}

/// A sound is named, never a path: letters, digits, spaces, `-` and `_`.
pub(super) fn plain(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
}

fn play(log: &Path, kind: &str, sound: &str, id: Option<&str>) -> Result<(), String> {
    let result = if plain(sound) {
        super::target::play_sound(sound)
    } else {
        Err("A sound is a plain name, not a path".to_owned())
    };
    let id = id.map(|id| format!(" id={id}")).unwrap_or_default();
    let line = match &result {
        Ok(()) => format!("played kind={kind} sound={sound}{id}"),
        Err(error) => format!("failed kind={kind} sound={sound}{id} error={error}"),
    };
    record(log, &line);
    result
}

fn record(log: &Path, line: &str) {
    let line = format!("{} {line}\n", chrono::Utc::now().to_rfc3339());
    eprint!("Agent HUD sound: {line}");
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if std::fs::metadata(log).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    super::private_output(&mut options);
    if let Err(error) = options
        .open(log)
        .and_then(|mut file| file.write_all(line.as_bytes()))
    {
        eprintln!("Agent HUD sound log: {error}");
    }
}

/// Plays a system sound by name now, for Settings to preview a choice.
#[tauri::command]
pub async fn native_play_sound(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Sounds>().preview(&name))
        .await
        .map_err(|_| "Sound worker failed".to_owned())?
}

#[cfg(test)]
#[path = "sound_tests.rs"]
mod tests;
