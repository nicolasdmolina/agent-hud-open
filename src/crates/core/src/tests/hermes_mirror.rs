//! A far machine's sessions, read from a mirror (`hud_mirror`) as Core
//! reads any Hermes `state.db`: their rows and cards carry the machine's
//! label, the label is saved with them, a new label reads them again
//! without counting anything twice, and a mirror that cannot be trusted
//! gives nothing. Every key and label here is made up.
use super::*;
use agent_hud_agents::hermes::gateway::{
    machine_key, Ask, Gateway, GatewayId, Machine, Pending, Profile,
};

/// What the made-up install id makes at the made-up far root home.
const KEY: &str = "aaeda597d860920c";

/// A Hermes `state.db` at `path` with `sessions`, each with one usage row
/// of `(input, output)`.
fn state_db(path: &Path, sessions: &[(&str, i64, i64)]) -> rusqlite::Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, started_at REAL NOT NULL, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0, title TEXT, profile_name TEXT);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL, billing_provider TEXT NOT NULL DEFAULT '', billing_base_url TEXT NOT NULL DEFAULT '', billing_mode TEXT NOT NULL DEFAULT '', task TEXT NOT NULL DEFAULT '', input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0, cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0, first_seen REAL, last_seen REAL, PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task));",
    )
    .unwrap();
    let now = Utc::now().timestamp();
    for (id, input, output) in sessions {
        db.execute(
            "INSERT INTO sessions VALUES (?1, 'telegram', 'claude-test', ?2, ?3, ?4, 'Synthetic', NULL)",
            rusqlite::params![id, now - 120, input, output],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_model_usage (session_id, model, input_tokens, output_tokens, first_seen, last_seen) VALUES (?1, 'claude-test', ?2, ?3, ?4, ?4)",
            rusqlite::params![id, input, output, now - 60],
        )
        .unwrap();
    }
    db
}

fn mirror(db: &rusqlite::Connection, machine: &str, label: &str) {
    db.execute_batch(
        "CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO hud_mirror VALUES (1, ?1, ?2, 'default')",
        [machine, label],
    )
    .unwrap();
}

/// This machine's Hermes home with `local:s1`, and a mirror folder as the
/// mirror writer lays one out, with the far machine's `s1` and `s2`.
struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    roots: Vec<(PathBuf, &'static str)>,
    far: rusqlite::Connection,
}

fn fixture(machine: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".hermes");
    state_db(&home.join("state.db"), &[("local-s1", 7, 1)]);
    let far_root = temp.path().join("hud/clients/hermes-far");
    let far = state_db(
        &far_root.join(format!("profiles/{KEY}-default/state.db")),
        &[("s1", 100, 20), ("s2", 30, 4)],
    );
    mirror(&far, machine, "Mac mini");
    Fixture {
        data: temp.path().join("app"),
        roots: vec![(home, "hermes"), (far_root, "hermes")],
        far,
        _temp: temp,
    }
}

impl Fixture {
    fn open(&self) -> AgentHudCore {
        let mut core = AgentHudCore::new(vec![], self.data.clone());
        core.provider_roots = self.roots.clone();
        core
    }
}

/// One whole collection; the ledger's total.
fn refresh(core: &mut AgentHudCore) -> Value {
    let mut snapshot = core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        if core.reads.pending.is_empty() {
            break;
        }
        snapshot = core.request("snapshot", json!({})).unwrap();
    }
    snapshot["totals"]["total"].clone()
}

/// Each listed Hermes session: its id, machine and tokens, sorted.
fn rows(core: &mut AgentHudCore, search: &str) -> Vec<(String, Value, Value)> {
    let found = core
        .request("sessions", json!({"scope": "all", "search": search}))
        .unwrap();
    let mut rows = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["source"] == "hermes")
        .map(|item| {
            assert!(item.get("navigationTarget").is_none(), "{item}");
            let id = item["id"].as_str().unwrap().to_owned();
            (id, item["machine"].clone(), item["tokens"].clone())
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn far(id: &str) -> String {
    format!("hermes:{KEY}/default:{id}")
}

/// The rows the fixture lists with the far machine shown as `label`.
fn listed(label: &str) -> Vec<(String, Value, Value)> {
    vec![
        (far("s1"), json!(label), json!(120)),
        (far("s2"), json!(label), json!(34)),
        ("hermes:local-s1".to_owned(), Value::Null, json!(8)),
    ]
}

/// Each saved Hermes session head and the machine it carries.
fn heads(core: &AgentHudCore) -> Vec<(String, Option<String>)> {
    let mut heads = core
        .logs
        .values()
        // Each database also has a head of its own, which is no session.
        .filter(|head| head.source == "hermes" && head.id.starts_with("hermes:"))
        .map(|head| (head.id.clone(), head.machine.clone()))
        .collect::<Vec<_>>();
    heads.sort();
    heads
}

#[test]
fn far_rows_carry_their_machine_and_a_new_label_counts_nothing_twice() {
    let fixture = fixture(KEY);
    let mut core = fixture.open();
    assert_eq!(refresh(&mut core), json!(162));
    assert_eq!(rows(&mut core, ""), listed("Mac mini"));
    // Search finds the label; this machine's row has none.
    assert_eq!(
        rows(&mut core, "mac mini")
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [far("s1"), far("s2")]
    );

    // Only the label changes: every far session is read again, under it,
    // and the ledger's total stays.
    fixture
        .far
        .execute("UPDATE hud_mirror SET label='Studio'", [])
        .unwrap();
    assert_eq!(refresh(&mut core), json!(162));
    assert_eq!(rows(&mut core, ""), listed("Studio"));
    assert!(rows(&mut core, "mac mini").is_empty());
    drop(core);

    // Relaunched: the saved heads carry the label before anything is read.
    let mut core = fixture.open();
    assert_eq!(
        heads(&core),
        [
            (far("s1"), Some("Studio".to_owned())),
            (far("s2"), Some("Studio".to_owned())),
            ("hermes:local-s1".to_owned(), None),
        ]
    );
    assert_eq!(rows(&mut core, ""), listed("Studio"));
    assert_eq!(refresh(&mut core), json!(162));
    // The persisted cursor still sees a new label after the relaunch.
    fixture
        .far
        .execute("UPDATE hud_mirror SET label='Lab'", [])
        .unwrap();
    assert_eq!(refresh(&mut core), json!(162));
    assert_eq!(rows(&mut core, ""), listed("Lab"));
}

#[test]
fn a_mirror_with_a_machine_that_is_no_key_gives_nothing() {
    for machine in [
        &KEY[..15],
        "AAEDA597D860920C",
        "0123456789abcdef0123456789abcdef",
    ] {
        let fixture = fixture(machine);
        let mut core = fixture.open();
        assert_eq!(refresh(&mut core), json!(8), "{machine}");
        assert_eq!(refresh(&mut core), json!(8), "{machine}");
        assert_eq!(
            rows(&mut core, ""),
            [("hermes:local-s1".to_owned(), Value::Null, json!(8))]
        );
        assert_eq!(heads(&core), [("hermes:local-s1".to_owned(), None)]);
        assert!(!core
            .logs
            .keys()
            .any(|path| path.to_string_lossy().contains("hermes-far")));
    }
}

#[test]
fn a_far_card_carries_its_machine_like_its_row() {
    let fixture = fixture(KEY);
    let mut core = fixture.open();
    refresh(&mut core);
    let key = machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    );
    let gateway = Gateway::new(GatewayId::new(Machine::Far(key.unwrap()), Profile::Default));
    let asked = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
    let (ask, wait_ms) = Ask::prompt(&asked).unwrap();
    let pending = Pending {
        session: gateway.session(&Profile::Default, "s1").unwrap(),
        ask,
        wait_ms,
    };
    let now = Utc::now().timestamp_millis();
    core.requests()
        .register_native(pending.id(), pending.view(now, Some("Mac mini")), now)
        .expect("a new card");
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let card = snapshot["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == pending.id().as_str())
        .expect("the card")
        .clone();
    assert_eq!(card["machine"], "Mac mini");
    assert_eq!(card["sessionId"], far("s1"));
    let row = rows(&mut core, "")
        .into_iter()
        .find(|row| row.0 == far("s1"))
        .unwrap();
    assert_eq!(row.1, card["machine"]);
}
