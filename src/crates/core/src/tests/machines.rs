//! The machines a native channel tells core it knows, carried to the
//! snapshot as told: one list for each source, replaced whole, and apart
//! from it the source's note that its list could not be read. Core decides
//! nothing from a state, a note or a time. No channel tells one yet; these
//! tests tell it as one will, under a source name no client has.
use super::*;
use agent_hud_agents::{MachineState, MachineStatus, Machines};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const SOURCE: &str = "far-client";
/// 2025-10-09T08:53:20Z.
const AT: i64 = 1_760_000_000_000;

const STATES: [(MachineState, &str); 6] = [
    (MachineState::Connected, "connected"),
    (MachineState::Connecting, "connecting"),
    (MachineState::NotTrusted, "not-trusted"),
    (MachineState::CannotReach, "cannot-reach"),
    (MachineState::NotCovered, "not-covered"),
    (MachineState::SameMachine, "same-machine"),
];

fn machine(id: &str, state: MachineState) -> MachineStatus {
    MachineStatus {
        id: id.into(),
        label: format!("Label {id}"),
        state,
        note: Some(format!("Note on {id}")),
        contact_ms: Some(AT),
        history_checked_ms: Some(AT - 60_000),
    }
}

fn told(list: Vec<MachineStatus>, note: Option<&str>) -> Machines {
    Machines {
        list,
        list_note: note.map(str::to_owned),
    }
}

/// A core with a Claude Code and a Codex session, and a count of the times
/// its requests woke the HUD.
fn core(home: &Path) -> (AgentHudCore, Arc<AtomicUsize>) {
    let at = Utc::now().to_rfc3339();
    let claude = home.join(".claude/projects/p");
    fs::create_dir_all(&claude).unwrap(); // plain directory
    let line = json!({"sessionId":"c1","timestamp":at,"type":"assistant","message":{"id":"m1","model":"claude-fable-5","usage":{"input_tokens":1,"output_tokens":1}}});
    fs::write(claude.join("c1.jsonl"), format!("{line}\n")).unwrap();
    let codex = home.join(".codex/sessions/2025/10/09");
    fs::create_dir_all(&codex).unwrap(); // plain directory
    let meta = json!({"timestamp":at,"type":"session_meta","payload":{"id":"x1","cwd":"/w"}});
    fs::write(codex.join("rollout-x1.jsonl"), format!("{meta}\n")).unwrap();
    let mut core = fixture(home);
    core.request("snapshot", json!({})).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let count = wakes.clone();
    core.requests.set_waker(Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    }));
    (core, wakes)
}

fn snapshot(core: &mut AgentHudCore) -> Value {
    core.request("snapshot", json!({})).unwrap()
}

#[test]
fn a_list_told_to_core_reaches_the_snapshot_with_every_field_in_each_state() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, _) = core(temp.path());
    let list = STATES.iter().enumerate();
    let list = list.map(|(n, (state, _))| machine(&format!("m{n}"), *state));
    core.requests
        .set_machines(SOURCE, &told(list.collect(), None));
    let snapshot = snapshot(&mut core);
    let shown = snapshot["machines"][SOURCE].as_array().unwrap();
    assert_eq!(shown.len(), STATES.len());
    for (n, (_, word)) in STATES.iter().enumerate() {
        assert_eq!(
            shown[n],
            json!({
                "id": format!("m{n}"),
                "label": format!("Label m{n}"),
                "state": word,
                "note": format!("Note on m{n}"),
                "lastContactAt": "2025-10-09T08:53:20+00:00",
                "historyCheckedAt": "2025-10-09T08:52:20+00:00",
            }),
            "machines come in the order told, every field as told"
        );
    }
    assert!(snapshot.get("machinesNote").is_none());
}

#[test]
fn an_absent_time_or_note_is_absent_from_the_snapshot_never_zero() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, _) = core(temp.path());
    let bare = MachineStatus {
        id: "m".into(),
        label: "Mini".into(),
        state: MachineState::Connecting,
        note: None,
        contact_ms: None,
        history_checked_ms: None,
    };
    core.requests.set_machines(SOURCE, &told(vec![bare], None));
    let snapshot = snapshot(&mut core);
    assert_eq!(
        snapshot["machines"][SOURCE],
        json!([{"id": "m", "label": "Mini", "state": "connecting"}])
    );
}

#[test]
fn the_same_list_told_again_bumps_nothing_and_wakes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, wakes) = core(temp.path());
    let first = told(vec![machine("m", MachineState::Connected)], Some("x"));
    core.requests.set_machines(SOURCE, &first);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    let revision = core.requests.revision();
    snapshot(&mut core);
    core.requests.set_machines(SOURCE, &first);
    assert_eq!(core.requests.revision(), revision);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    let again = core.request("request_snapshot", json!({"changesOnly": true}));
    assert_eq!(again.unwrap(), json!({"unchanged": true}));

    // Each changed field is a new list: one revision and one wake each.
    let changes: [fn(&mut Machines); 7] = [
        |m| m.list[0].label = "Mini 2".into(),
        |m| m.list[0].state = MachineState::CannotReach,
        |m| m.list[0].note = None,
        |m| m.list[0].contact_ms = Some(AT + 1_000),
        |m| m.list[0].history_checked_ms = None,
        |m| m.list_note = Some("y".into()),
        |m| m.list.push(machine("n", MachineState::NotCovered)),
    ];
    let mut next = first;
    for (n, change) in changes.iter().enumerate() {
        let revision = core.requests.revision();
        change(&mut next);
        core.requests.set_machines(SOURCE, &next);
        assert_eq!(core.requests.revision(), revision.wrapping_add(1), "{n}");
        assert_eq!(wakes.load(Ordering::SeqCst), n + 2, "change {n} wakes once");
        let shown = core.request("request_snapshot", json!({"changesOnly": true}));
        assert!(
            shown.unwrap().get("unchanged").is_none(),
            "change {n} shows"
        );
    }
}

#[test]
fn an_empty_list_and_a_source_that_never_told_give_no_machines() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, wakes) = core(temp.path());
    core.requests.set_machines(SOURCE, &Machines::default());
    assert_eq!(wakes.load(Ordering::SeqCst), 0, "nothing told, nothing new");
    let other = "near-client";
    let list = vec![machine("m", MachineState::Connected)];
    core.requests.set_machines(other, &told(list, None));
    let shown = snapshot(&mut core);
    assert!(shown["machines"].get(SOURCE).is_none());
    assert_eq!(shown["machines"][other].as_array().unwrap().len(), 1);

    // A list told and then emptied leaves nothing for that source.
    core.requests.set_machines(other, &Machines::default());
    let shown = snapshot(&mut core);
    assert!(shown.get("machines").is_none());
    assert!(shown.get("machinesNote").is_none());
}

#[test]
fn the_note_on_the_list_travels_beside_it_and_goes_when_none_is_told() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, _) = core(temp.path());
    // No list known yet, and the list could not be read.
    core.requests
        .set_machines(SOURCE, &told(vec![], Some("List unread")));
    let shown = snapshot(&mut core);
    assert!(shown.get("machines").is_none(), "an empty list stays empty");
    assert_eq!(shown["machinesNote"], json!({SOURCE: "List unread"}));

    // The channel keeps its last list and says the newer one was unreadable.
    let list = vec![machine("m", MachineState::Connected)];
    core.requests
        .set_machines(SOURCE, &told(list.clone(), Some("List unread")));
    let shown = snapshot(&mut core);
    assert_eq!(shown["machines"][SOURCE][0]["id"], "m");
    assert_eq!(shown["machinesNote"][SOURCE], "List unread");

    core.requests.set_machines(SOURCE, &told(list, None));
    let shown = snapshot(&mut core);
    assert_eq!(shown["machines"][SOURCE][0]["id"], "m");
    assert!(shown.get("machinesNote").is_none());
}

#[test]
fn clear_drops_every_sources_machines_and_note() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, wakes) = core(temp.path());
    let list = vec![machine("m", MachineState::Connected)];
    core.requests
        .set_machines(SOURCE, &told(list, Some("List unread")));
    let revision = core.requests.revision();
    core.requests.clear();
    assert_eq!(core.requests.revision(), revision.wrapping_add(1));
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    let shown = snapshot(&mut core);
    assert!(shown.get("machines").is_none());
    assert!(shown.get("machinesNote").is_none());
}

#[test]
fn a_snapshot_no_channel_told_machines_has_exactly_the_keys_it_had() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, _) = core(temp.path());
    let shown = snapshot(&mut core);
    let sessions = shown["sessions"].as_array().unwrap();
    for source in ["claude", "codex"] {
        assert!(
            sessions.iter().any(|row| row["source"] == source),
            "{source}"
        );
    }
    let mut keys: Vec<&str> = shown
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "accounts",
            "approvals",
            "billing",
            "capabilities",
            "connection",
            "generatedAt",
            "indexing",
            "islandEvents",
            "language",
            "mode",
            "providers",
            "quotas",
            "relays",
            "requestDeliveries",
            "schemaVersion",
            "sessionScope",
            "sessions",
            "sessionsRevision",
            "settings",
            "soundAlerts",
            "statisticsRevision",
            "timeline",
            "totals",
        ]
    );
}
