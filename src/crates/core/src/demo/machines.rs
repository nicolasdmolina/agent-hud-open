//! What a demo script's `machines` step tells core, and the settings a
//! script starts with: a source's machines in the snapshot's shape, read
//! into what a native channel would tell, with times given as seconds before
//! the step so a take reads the same whenever it is played.
use crate::AgentHudCore;
use agent_hud_agents::{MachineState, MachineStatus, Machines};
use serde_json::Value;
use std::collections::BTreeMap;

/// The machines a `machines` step names, as told at `now_ms`: `list` (each
/// with `id`, `label`, `state` as the snapshot writes it, and optionally
/// `note`, `here`, `contactAgo`, `historyAgo`) and `note`, the list's own.
pub fn demo_machines(step: &Value, now_ms: i64) -> Result<Machines, String> {
    let ago = |item: &Value, key: &str| {
        item[key]
            .as_f64()
            .map(|seconds| now_ms - (seconds * 1000.0) as i64)
    };
    let text = |item: &Value, key: &str| {
        item[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("A demo machine needs \"{key}\""))
    };
    let mut list = Vec::new();
    for item in step["list"].as_array().into_iter().flatten() {
        let word = text(item, "state")?;
        list.push(MachineStatus {
            id: text(item, "id")?,
            label: text(item, "label")?,
            state: MachineState::from_word(&word)
                .ok_or_else(|| format!("No machine state \"{word}\""))?,
            note: item["note"].as_str().map(str::to_owned),
            contact_ms: ago(item, "contactAgo"),
            history_checked_ms: ago(item, "historyAgo"),
            here: item["here"].as_bool().unwrap_or(false),
        });
    }
    Ok(Machines {
        list,
        list_note: step["note"].as_str().map(str::to_owned),
    })
}

impl AgentHudCore {
    /// Sets each of a demo script's settings as the Settings window would.
    pub fn demo_settings(&mut self, settings: &BTreeMap<String, Value>) -> Result<(), String> {
        for (key, value) in settings {
            self.apply_setting(key, value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_step_reads_into_what_a_channel_tells_with_times_before_it() {
        let step = json!({"source": "x", "note": "Unread", "list": [
            {"id": "a", "label": "Studio", "state": "connected", "contactAgo": 42, "historyAgo": 3600},
            {"id": "b", "label": "Here", "state": "same-machine", "note": "Studio", "here": true}
        ]});
        let told = demo_machines(&step, 1_000_000).unwrap();
        assert_eq!(told.list_note.as_deref(), Some("Unread"));
        assert_eq!(told.list[0].state, MachineState::Connected);
        assert_eq!(told.list[0].contact_ms, Some(958_000));
        assert_eq!(told.list[0].history_checked_ms, Some(1_000_000 - 3_600_000));
        assert!(!told.list[0].here);
        assert_eq!(told.list[1].contact_ms, None);
        assert_eq!(told.list[1].note.as_deref(), Some("Studio"));
        assert!(told.list[1].here);
        let wrong = json!({"list": [{"id": "a", "label": "A", "state": "asleep"}]});
        assert!(demo_machines(&wrong, 0).is_err());
    }
}
