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

/// The Hermes events in the ledger itself: how many, and their tokens.
fn ledger(core: &AgentHudCore) -> (i64, i64) {
    let sql = "SELECT COUNT(*), TOTAL(input+output+cache_read+cache_write) FROM usage_meter_event WHERE source='hermes'";
    let read = |row: &rusqlite::Row| Ok((row.get(0)?, row.get::<_, f64>(1)? as i64));
    core.database
        .as_ref()
        .unwrap()
        .query_row(sql, [], read)
        .unwrap()
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
    // The ledger itself: its Hermes events and their tokens.
    let events = ledger(&core);
    assert_eq!(events.1, 162);
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
    assert_eq!(ledger(&core), events);
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
    assert_eq!(ledger(&core), events);
    // The persisted cursor still sees a new label after the relaunch.
    fixture
        .far
        .execute("UPDATE hud_mirror SET label='Lab'", [])
        .unwrap();
    assert_eq!(refresh(&mut core), json!(162));
    assert_eq!(rows(&mut core, ""), listed("Lab"));
    assert_eq!(ledger(&core), events);
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

/// What the mirror writer writes for a session a poll listed
/// (`hermes::mirror::claim` and `list`, the functions it calls) is a row
/// and nothing else: no tokens on it, no event in the ledger, and
/// statistics as they were.
#[test]
fn a_row_made_from_the_live_list_adds_nothing_to_the_ledger() {
    use agent_hud_agents::hermes::{gateway::Listed, mirror as written};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".hermes");
    state_db(&home.join("state.db"), &[("local-s1", 7, 1)]);
    let far_root = temp.path().join("hud").join(written::ROOT);
    let key = machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap();
    assert_eq!(key.as_str(), KEY);
    // The mirror as the writer makes one, with a durable row put in by
    // hand as the far database gives one: 120 tokens, by model.
    let path = written::file(&far_root, &key, &Profile::Default);
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = rusqlite::Connection::open(&path).unwrap();
    assert!(written::claim(&db, &key, &Profile::Default, "Mac mini").unwrap());
    let now = Utc::now().timestamp();
    db.execute_batch(&format!(
        "INSERT INTO sessions (id, source, model, started_at, input_tokens, output_tokens, title)
            VALUES ('s1', 'telegram', 'claude-test', {0}, 100, 20, 'Durable');
         INSERT INTO session_model_usage (session_id, model, input_tokens, output_tokens, first_seen, last_seen)
            VALUES ('s1', 'claude-test', 100, 20, {0}, {0});
         INSERT INTO hud_rows VALUES ('s1', 1, 1, 'print', 0);",
        now - 60
    ))
    .unwrap();
    let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
    core.provider_roots = vec![(home, "hermes"), (far_root, "hermes")];
    assert_eq!(refresh(&mut core), json!(128));
    let events = ledger(&core);
    assert_eq!(events.1, 128);
    let iso = |at: i64| {
        chrono::DateTime::<Utc>::from_timestamp(at, 0)
            .unwrap()
            .to_rfc3339()
    };
    let (from, to) = (iso(now - 30 * 86_400), iso(now + 86_400));
    let query = json!({"from": from, "to": to, "bucketMinutes": 1440, "costKinds": "all",
        "tokenKinds": ["input", "output", "cacheWrite"], "timeZone": "UTC"});
    // Everything statistics say, less the clock: when the answer was
    // worked out, which reading of the ledger it is, and up to when the
    // ledger has looked (`observedTo`), none of which is usage.
    fn timeless(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for clock in ["generatedAt", "dataRevision", "observedTo"] {
                    map.remove(clock);
                }
                map.values_mut().for_each(timeless);
            }
            Value::Array(items) => items.iter_mut().for_each(timeless),
            _ => {}
        }
    }
    let statistics = |core: &mut AgentHudCore| {
        let mut all = core.request("statistics", query.clone()).unwrap();
        timeless(&mut all);
        all
    };
    let before = statistics(&mut core);
    assert!(before.to_string().contains("128"), "{before}");

    // A poll lists a session the mirror lacks, and the durable one,
    // said otherwise.
    let said = |title: &str| Listed {
        title: Some(title.into()),
        model: Some("claude-test".into()),
        source: Some("telegram".into()),
        started_at: Some((now - 30) as f64),
    };
    let listed = [
        ("live1".to_owned(), said("Listed live")),
        ("s1".to_owned(), said("Said otherwise")),
    ];
    assert_eq!(written::list(&db, &listed, now * 1000).unwrap(), 1);

    assert_eq!(refresh(&mut core), json!(128));
    assert_eq!(
        rows(&mut core, ""),
        [
            (far("live1"), json!("Mac mini"), json!(0)),
            (far("s1"), json!("Mac mini"), json!(120)),
            ("hermes:local-s1".to_owned(), Value::Null, json!(8)),
        ]
    );
    // The ledger itself: not one event more, and the same tokens.
    assert_eq!(ledger(&core), events);
    assert_eq!(statistics(&mut core), before);
    // Nor after a relaunch, which reads the mirror from what was saved.
    drop(core);
    let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
    core.provider_roots = vec![
        (temp.path().join(".hermes"), "hermes"),
        (temp.path().join("hud").join(written::ROOT), "hermes"),
    ];
    assert_eq!(refresh(&mut core), json!(128));
    assert_eq!(ledger(&core), events);
    assert_eq!(statistics(&mut core), before);
}

/// Ticket 54: a mirror core read while it had no table yet is left be
/// for 30 s and longer, like any log that cannot be read. A file event
/// does not end that. Its writer saying it wrote it does: the next pass
/// reads it.
#[test]
fn a_file_its_writer_says_it_wrote_is_read_though_its_last_read_failed() {
    let fixture = fixture(KEY);
    let path = fixture.roots[1]
        .0
        .join(format!("profiles/{KEY}-reports/state.db"));
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let made = rusqlite::Connection::open(&path).unwrap();
    made.pragma_update(None, "journal_mode", "WAL").unwrap();
    drop(made);
    let mut core = fixture.open();
    let before = Utc::now().timestamp_millis();
    refresh(&mut core);
    // Read, which failed: it is not tried again for 30 s.
    let paused = core.reads.failed.retry_at(&path).expect("a failed read");
    assert!(paused >= before + 30_000);
    assert!(!core.indexing(), "a pausing log is not waited for");

    // The writer's first commit.
    let db = state_db(&path, &[("r1", 5, 1)]);
    db.execute_batch(&format!(
        "CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);
         INSERT INTO hud_mirror VALUES (1, '{KEY}', 'Mac mini', 'reports');"
    ))
    .unwrap();
    let wanted = format!("hermes:{KEY}/reports:r1");
    let listed = |core: &mut AgentHudCore| rows(core, "").iter().any(|row| row.0 == wanted);

    // The file watcher reports the write: the pause stands.
    core.file_changes(Some(HashSet::from([path.clone()])));
    core.request("snapshot", json!({})).unwrap();
    assert!(!listed(&mut core));
    assert_eq!(core.reads.failed.retry_at(&path), Some(paused));

    // The writer says it wrote the file.
    core.written_files(HashSet::from([path.clone()]));
    assert!(core.indexing(), "it is waited for again");
    assert!(core.next_pass_due().unwrap() <= std::time::Instant::now());
    core.request("snapshot", json!({})).unwrap();
    assert!(listed(&mut core));
    assert_eq!(core.reads.failed.retry_at(&path), None);
    // A file that was never paused is told like any change.
    core.written_files(HashSet::from([path.clone()]));
    core.request("snapshot", json!({})).unwrap();
    assert!(listed(&mut core));
}

/// And a read that fails again after its writer's word pauses again,
/// for longer.
#[test]
fn a_told_file_that_still_cannot_be_read_pauses_again_for_longer() {
    let fixture = fixture(KEY);
    let path = fixture.roots[1]
        .0
        .join(format!("profiles/{KEY}-reports/state.db"));
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let made = rusqlite::Connection::open(&path).unwrap();
    made.pragma_update(None, "journal_mode", "WAL").unwrap();
    let mut core = fixture.open();
    refresh(&mut core);
    let before = Utc::now().timestamp_millis();
    core.written_files(HashSet::from([path.clone()]));
    core.request("snapshot", json!({})).unwrap();
    let after = Utc::now().timestamp_millis();
    // The second failure in a row: 60 s.
    let paused = core.reads.failed.retry_at(&path).expect("a failed read");
    assert!((before + 60_000..=after + 60_000).contains(&paused));
}

/// Each Hermes row's `origins`: where the session was started, from its
/// stored `source` (ticket 39).
fn origins(core: &mut AgentHudCore) -> Vec<(String, Value)> {
    let found = core.request("sessions", json!({"scope": "all"})).unwrap();
    let mut rows = (found["items"].as_array().unwrap().iter())
        .filter(|item| item["source"] == "hermes")
        .map(|item| {
            (
                item["id"].as_str().unwrap().to_owned(),
                item["origins"].clone(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn both(tag: &str) -> Value {
    json!({"init": tag, "latest": tag})
}

/// A session the Hermes desktop app runs says it was started there, on
/// its row, its page and after a relaunch; every other says where it was
/// (ticket 39). With Hermes's app link registered, it stands in as the
/// way back of every Hermes session that has none better, named for what
/// it does, and of no other client's.
#[test]
fn hermes_rows_say_where_they_were_started_and_open_in_the_desktop_app() {
    let fixture = fixture(KEY);
    (fixture.far)
        .execute("UPDATE sessions SET source='desktop' WHERE id='s1'", [])
        .unwrap();
    let mut core = fixture.open();
    refresh(&mut core);
    let expected = vec![
        (far("s1"), both("desktop:hermes")),
        (far("s2"), both("others:telegram")),
        ("hermes:local-s1".to_owned(), both("others:telegram")),
    ];
    assert_eq!(origins(&mut core), expected);
    let detail = (core.request("session_detail", json!({"id": far("s1")}))).unwrap();
    assert_eq!(detail["session"]["origins"], both("desktop:hermes"));
    assert!(detail["session"].get("navigationTarget").is_none());
    drop(core);

    // Relaunched: the saved heads say it before anything is read.
    let mut core = fixture.open();
    assert_eq!(origins(&mut core), expected);

    let app = json!({"kind": "hermes-app"});
    core.set_provider_discovery(vec![crate::ProviderDiscovery {
        id: "hermes".into(),
        navigation_target: Some(app.clone()),
        ..Default::default()
    }]);
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let hermes = (snapshot["sessions"].as_array().unwrap().iter())
        .filter(|session| session["source"] == "hermes")
        .collect::<Vec<_>>();
    assert_eq!(hermes.len(), 3, "{snapshot}");
    for session in hermes {
        assert_eq!(session["navigationTarget"], app, "{session}");
        assert_eq!(session["navigationLabel"], "Open in Hermes desktop");
    }
    let detail = (core.request("session_detail", json!({"id": far("s1")}))).unwrap();
    assert_eq!(detail["session"]["navigationTarget"], app);
    assert_eq!(
        detail["session"]["navigationLabel"],
        "Open in Hermes desktop"
    );
    let page = core.request("sessions", json!({"scope": "all"})).unwrap();
    for item in page["items"].as_array().unwrap() {
        assert_eq!(item["navigationLabel"], "Open in Hermes desktop", "{item}");
    }
    // A link of another kind than Hermes's app link is no way back that
    // name fits: it is called as any other.
    let other = json!({"kind": "hermes-other"});
    core.set_provider_discovery(vec![crate::ProviderDiscovery {
        id: "hermes".into(),
        navigation_target: Some(other.clone()),
        ..Default::default()
    }]);
    let detail = (core.request("session_detail", json!({"id": far("s1")}))).unwrap();
    assert_eq!(detail["session"]["navigationTarget"], other);
    assert!(detail["session"].get("navigationLabel").is_none(), "{detail}");
}
