//! A wait with no card that only a channel's waiting flag tells, seen to
//! end, and a new one a day and a millisecond later with nothing written:
//! the later wait gets an alert id of its own, through the whole core
//! (Agent HUD Fork, ticket 75; the probe of ticket 66's round 4 verifier).
use super::*;
use agent_hud_agents::SOLE_READER;
use std::collections::BTreeSet;

const THREAD: &str = "00000000-0000-4000-8000-000000000075";

/// The `needs-you` ids the snapshot carries, with repeats.
fn look(core: &mut AgentHudCore, now: i64) -> Vec<String> {
    crate::snapshot::PINNED_CLOCK.set(Some(now));
    let snapshot = core.request("request_snapshot", json!({})).unwrap();
    let items = snapshot["soundAlerts"]["items"].as_array().unwrap().clone();
    let needs = items.iter().filter(|item| item["kind"] == "needs-you");
    needs
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

fn waits(core: &AgentHudCore, on: bool, now: i64) {
    let sessions = if on { vec![THREAD.to_owned()] } else { vec![] };
    core.requests()
        .set_native_waiting("codex", SOLE_READER, &sessions, now);
}

#[test]
fn a_new_wait_a_day_after_the_first_cleared_sounds_under_a_new_id() {
    let temp = tempfile::tempdir().unwrap();
    let mut now = Utc::now().timestamp_millis();
    let folder = temp.path().join(".codex/sessions");
    fs::create_dir_all(&folder).unwrap();
    let lines = [
        json!({"timestamp":crate::iso(now-3000),"type":"session_meta","payload":{"id":THREAD,"timestamp":crate::iso(now-3000),"source":"vscode","originator":"Codex Desktop","cwd":"/synthetic"}}),
        json!({"timestamp":crate::iso(now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-75"}}),
    ];
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    fs::write(folder.join(format!("rollout-{THREAD}.jsonl")), text).unwrap();
    let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
    core.provider_roots = vec![(folder, "codex")];
    core.apply_setting("sound", &json!(true)).unwrap();
    crate::snapshot::PINNED_CLOCK.set(Some(now));
    core.request("refresh", json!({})).unwrap();
    look(&mut core, now);
    waits(&core, true, now);
    let first = look(&mut core, now);
    assert_eq!(first.len(), 1, "{first:?}");
    now += 100;
    waits(&core, false, now);
    look(&mut core, now);
    now += 86_400_001;
    look(&mut core, now);
    now += 1;
    waits(&core, true, now);
    let raw = look(&mut core, now);
    // It only goes on: no other.
    let again = look(&mut core, now + 1_000);
    crate::snapshot::PINNED_CLOCK.set(None);
    let distinct: BTreeSet<&String> = raw.iter().collect();
    let new = distinct.iter().filter(|id| !first.contains(**id)).count();
    assert_eq!(
        (raw.len(), distinct.len(), new),
        (2, 2, 1),
        "{first:?} then {raw:?}"
    );
    assert_eq!(again, raw);
}
