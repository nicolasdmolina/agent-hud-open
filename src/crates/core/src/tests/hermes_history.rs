//! A far machine's stored history in the ledger (ticket 37): what the
//! far database gave is written into a mirror as the mirror writer writes
//! it (`agents::hermes::mirror::durable`), and Core reads that mirror as
//! it reads any Hermes `state.db`. Its usage is in the ledger once,
//! whatever came first: a row from the live list, the same batch again,
//! or a later one in which the session grew. Every key, label and session
//! is made up.
use super::*;
use agent_hud_agents::hermes::{
    far_history::{Cell, Session},
    gateway::{machine_key, Listed, MachineKey, Profile},
    mirror,
};

fn key() -> MachineKey {
    machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap()
}

/// A far session as the far reader gives it: one usage row for each of
/// `usage`, by model, task and input tokens, with 20 output tokens each.
fn far_session(id: &str, usage: &[(&str, &str, i64)], print: &str) -> Session {
    let text = |text: &str| Cell::Text(text.into());
    let now = Utc::now().timestamp() as f64;
    let row = |(model, task, input): &(&str, &str, i64)| {
        vec![
            text(model),
            text("openrouter"),
            text("https://openrouter.ai/api/v1"),
            text("api"),
            text(task),
            Cell::Int(*input),
            Cell::Int(20),
            Cell::Int(0),
            Cell::Int(0),
            Cell::Real(now - 90.0),
            Cell::Real(now - 60.0),
        ]
    };
    let input: i64 = usage.iter().map(|row| row.2).sum();
    Session {
        rowid: 1,
        id: id.to_owned(),
        print: print.to_owned(),
        cells: vec![
            Cell::Real(now - 120.0),
            text("Synthetic"),
            Cell::Null,
            text("telegram"),
            text("claude-test"),
            text("anthropic"),
            text("https://api.anthropic.com"),
            text("api"),
            Cell::Int(input),
            Cell::Int(20 * usage.len() as i64),
            Cell::Int(0),
            Cell::Int(0),
            Cell::Real(now - 30.0),
            Cell::Real(now - 30.0),
            Cell::Null,
        ],
        usage: usage.iter().map(row).collect(),
    }
}

struct Far {
    _temp: tempfile::TempDir,
    data: PathBuf,
    root: PathBuf,
    db: rusqlite::Connection,
}

/// A mirror of the default profile as the writer makes one, empty.
fn far() -> Far {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("hud").join(mirror::ROOT);
    let path = mirror::file(&root, &key(), &Profile::Default);
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = rusqlite::Connection::open(&path).unwrap();
    assert!(mirror::claim(&db, &key(), &Profile::Default, "Mac mini").unwrap());
    Far {
        data: temp.path().join("app"),
        root,
        db,
        _temp: temp,
    }
}

impl Far {
    fn open(&self) -> AgentHudCore {
        let mut core = AgentHudCore::new(vec![], self.data.clone());
        core.provider_roots = vec![(self.root.clone(), "hermes")];
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

/// The ledger itself: each model's events and tokens, sorted.
fn ledger(core: &AgentHudCore) -> Vec<(String, i64, i64)> {
    let sql =
        "SELECT model, COUNT(*), TOTAL(input+output+cache_read+cache_write) FROM usage_meter_event
        WHERE source='hermes' GROUP BY model ORDER BY model";
    let database = core.database.as_ref().unwrap();
    let mut statement = database.prepare(sql).unwrap();
    let rows = statement.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get::<_, f64>(2)? as i64))
    });
    rows.unwrap().map(Result::unwrap).collect()
}

/// The tokens Core lists for the far session `id`.
fn tokens(core: &mut AgentHudCore, id: &str) -> Value {
    let found = core
        .request("sessions", json!({"scope": "all", "search": ""}))
        .unwrap();
    let id = format!("hermes:{}/default:{id}", key().as_str());
    let items = found["items"].as_array().unwrap();
    let row = items.iter().find(|item| item["id"] == id.as_str());
    row.map_or(Value::Null, |row| row["tokens"].clone())
}

#[test]
fn a_far_sessions_durable_rows_are_in_the_ledger_once_by_model_and_task() {
    let far = far();
    let ended = far_session(
        "s-ended",
        &[
            ("model-a", "", 100),
            ("model-b", "", 60),
            ("model-a", "aux:title", 5),
        ],
        "print-1",
    );
    assert_eq!(
        mirror::durable(&far.db, std::slice::from_ref(&ended), 1_000).unwrap(),
        1
    );
    let mut core = far.open();
    // 165 input and 60 output, split by model as the far rows are.
    assert_eq!(refresh(&mut core), json!(225));
    assert_eq!(tokens(&mut core, "s-ended"), json!(225));
    let events = ledger(&core);
    let by_model: Vec<(&str, i64)> = events.iter().map(|row| (row.0.as_str(), row.2)).collect();
    assert_eq!(by_model.iter().map(|row| row.1).sum::<i64>(), 225);
    assert_eq!(by_model.len(), 2, "{events:?}");
    assert!(
        by_model[0].0.starts_with("model-a") && by_model[0].1 == 145,
        "{events:?}"
    );
    assert!(
        by_model[1].0.starts_with("model-b") && by_model[1].1 == 80,
        "{events:?}"
    );

    // The same batch read again later: the rows are written again as
    // they were, and nothing is counted twice.
    assert_eq!(
        mirror::durable(&far.db, std::slice::from_ref(&ended), 2_000).unwrap(),
        1
    );
    assert_eq!(refresh(&mut core), json!(225));
    assert_eq!(ledger(&core), events);

    // The session grew on the far machine: one row by 150, and a new
    // auxiliary task. Only the growth is added.
    let grown = far_session(
        "s-ended",
        &[
            ("model-a", "", 250),
            ("model-b", "", 60),
            ("model-a", "aux:title", 5),
            ("model-b", "aux:compress", 9),
        ],
        "print-2",
    );
    assert_eq!(mirror::durable(&far.db, &[grown], 3_000).unwrap(), 1);
    assert_eq!(refresh(&mut core), json!(225 + 150 + 29));
    assert_eq!(tokens(&mut core, "s-ended"), json!(404));
    let after: i64 = ledger(&core).iter().map(|row| row.2).sum();
    assert_eq!(after, 404);
    drop(core);

    // Relaunched on the same ledger: read again from the saved cursor,
    // and still once.
    let mut core = far.open();
    assert_eq!(refresh(&mut core), json!(404));
    assert_eq!(ledger(&core).iter().map(|row| row.2).sum::<i64>(), 404);
    // An older read that comes back late changes nothing.
    assert_eq!(mirror::durable(&far.db, &[ended], 2_500).unwrap(), 0);
    assert_eq!(refresh(&mut core), json!(404));
}

#[test]
fn a_session_first_listed_live_and_then_read_from_the_database_is_counted_once() {
    let far = far();
    // Listed live: a row with the model the list gave and no counts.
    let said = Listed {
        title: Some("From the live list".into()),
        model: Some("model-a".into()),
        source: Some("telegram".into()),
        started_at: Some(Utc::now().timestamp() as f64 - 120.0),
    };
    assert_eq!(
        mirror::list(&far.db, &[("s-live".to_owned(), said)], 1_000).unwrap(),
        1
    );
    let mut core = far.open();
    assert_eq!(refresh(&mut core), json!(0));
    assert_eq!(tokens(&mut core, "s-live"), json!(0));
    assert!(ledger(&core).is_empty());

    // Read from the far database while it runs, and again once it grew:
    // each time the ledger holds what the far rows say, once.
    for (input, print, read_ms) in [(100, "print-1", 2_000), (400, "print-2", 3_000)] {
        let session = far_session("s-live", &[("model-a", "", input)], print);
        assert_eq!(mirror::durable(&far.db, &[session], read_ms).unwrap(), 1);
        assert_eq!(refresh(&mut core), json!(input + 20));
        assert_eq!(tokens(&mut core, "s-live"), json!(input + 20));
        let events = ledger(&core);
        assert_eq!(
            events.iter().map(|row| row.2).sum::<i64>(),
            input + 20,
            "{events:?}"
        );
    }
    // A live list that names it again changes nothing.
    assert_eq!(
        mirror::list(&far.db, &[("s-live".to_owned(), Listed::default())], 4_000).unwrap(),
        0
    );
    assert_eq!(refresh(&mut core), json!(420));
}
