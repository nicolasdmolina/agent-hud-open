//! One Hermes session is one row, whichever of its three sources names
//! it: the `state.db` reader, a hook, and the gateway's card. The first
//! review of ticket 26 (`evidence/26/verifier-files/identity_core.rs`)
//! showed a root that lies at `…/profiles/<name>` split into two rows:
//! the reader and the hook named its session `hermes:<name>:<id>`, and the
//! gateway, which knew the root, `hermes:<id>`.
//!
//! This is that probe. Three things in it are changed, none an
//! assertion: its scratch folder is a temporary one, the schema is read
//! from the crate's own fixtures, and `identify` is no longer told the
//! root, since the second review showed that Hermes itself names such a
//! home by its path.
use super::*;
use agent_hud_agents::hermes::gateway::{
    identify, Ask, GatewayId, Identified, Machine, Pending, Profile, SessionRef, SESSION_AUTHORITY,
};
use agent_hud_agents::parse_event;
use std::time::Duration;

const SCHEMA: &str = include_str!("../../../agents/tests/fixtures/hermes/macos/state.schema.sql");

#[test]
fn a_profile_shaped_root_is_one_row_with_its_card() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("profiles/work");
    fs::create_dir_all(&root).unwrap();
    let dbpath = root.join("state.db");
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    db.execute_batch(SCHEMA).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let started = (now - 600_000) as f64 / 1000.0;
    db.execute(
        "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES('s1','cron','m',?1,?1)",
        [started],
    )
    .unwrap();
    drop(db);
    let source = SourceRoot {
        path: dbpath,
        provider: "hermes",
    };
    let mut core = AgentHudCore::new(vec![source], temp.path().join("hud"));
    let stored = |snapshot: &Value| {
        let rows = snapshot["sessions"].as_array().unwrap();
        rows.iter().any(|row| row["id"] == "hermes:work:s1")
    };
    let mut snapshot = core.request("refresh", json!({})).unwrap();
    for _ in 0..200 {
        if stored(&snapshot) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        snapshot = core.request("snapshot", json!({})).unwrap();
    }
    assert!(stored(&snapshot));
    let home = root.to_str().unwrap();
    let answer = json!({"runtime_protocol":1,"state":"ready","instance_id":"synthetic",
        "authority_epoch":1,"api_origin":"http://127.0.0.1:54321",
        "capabilities":[SESSION_AUTHORITY],
        "served_profiles":[{"profile_id":home,"home":home}]});
    let Identified::Ready(instance) = identify(&answer, home) else {
        panic!("ready")
    };
    let session = SessionRef::new(
        GatewayId::new(Machine::This, Profile::Default),
        instance.served[0].profile.clone(),
        "s1",
    )
    .unwrap();
    let prompt = json!({"kind":"clarify","prompt_id":"p1","execution_generation":1,
        "question":"Synthetic question","choices":["one","two"],"multi_select":false,
        "timeout_s":600});
    let (ask, wait_ms) = Ask::prompt(&prompt).unwrap();
    let pending = Pending {
        session,
        ask,
        wait_ms,
    };
    let requests = core.requests();
    let _ticket = requests
        .register_native(pending.id(), pending.view(now, None), now)
        .unwrap();
    requests.set_native_waiting("hermes", "local", &[pending.session.hud_id()], now);
    let mut payload = json!({"hook_event_name":"pre_llm_call","session_id":"s1"});
    agent_hud_agents::hooks::add_environment("hermes", &mut payload, &|key| {
        (key == "HERMES_HOME").then(|| home.to_owned())
    });
    let hook = parse_event("hermes", "--completion-hook", &payload, now);
    core.ingest_hook_events(vec![hook.unwrap().unwrap()]);
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let rows: Vec<Value> = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["source"] == "hermes")
        .map(|row| json!({"id":row["id"],"state":row["state"],"evidence":row["evidence"],"requests":row["liveFacts"]["requests"]}))
        .collect();
    eprintln!("CORE ROWS {rows:?}");
    eprintln!("CARD {}", pending.view(now, None));
    assert_eq!(
        rows.len(),
        1,
        "the same Hermes session split into a stored/hook row and a native-card row"
    );
    // And the one row is the stored one, with the card on it.
    assert_eq!(rows[0]["id"], "hermes:work:s1");
    assert_eq!(rows[0]["requests"][0]["id"], pending.id());
    assert_eq!(pending.id(), "hermes:local:work:s1:p1");
}
