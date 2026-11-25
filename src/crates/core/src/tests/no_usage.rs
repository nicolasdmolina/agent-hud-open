//! Sessions that used no AI (ticket 70): a far machine's scheduled job that
//! ran no model stays out of the panel's short list and sounds no
//! `finished`, while it neither waits nor runs and until it uses tokens. The
//! Sessions page keeps it. Every key, id and label here is made up.
use super::*;
use agent_hud_agents::HookEvent;

/// What the made-up install id makes at the made-up far root home.
const KEY: &str = "aaeda597d860920c";

fn far(id: &str) -> String {
    format!("hermes:{KEY}/default:{id}")
}

/// A Hermes `state.db` with a session that worked and one that used nothing,
/// as the Mini's hourly job does: no tokens and no usage rows.
fn state_db(path: &Path, counted: bool) -> rusqlite::Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, started_at REAL NOT NULL, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0, title TEXT, profile_name TEXT);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL, billing_provider TEXT NOT NULL DEFAULT '', billing_base_url TEXT NOT NULL DEFAULT '', billing_mode TEXT NOT NULL DEFAULT '', task TEXT NOT NULL DEFAULT '', input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0, cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0, first_seen REAL, last_seen REAL, PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task));
         CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO hud_mirror VALUES (1, ?1, 'Mac mini', 'default')",
        [KEY],
    )
    .unwrap();
    let now = Utc::now().timestamp();
    for (id, title, at) in [
        ("worked", "Telegram chat", now - 240),
        ("job", "Hourly job", now - 120),
    ] {
        db.execute(
            "INSERT INTO sessions VALUES (?1, 'cron', 'claude-test', ?2, 0, 0, ?3, NULL)",
            rusqlite::params![id, at, title],
        )
        .unwrap();
    }
    if counted {
        use_tokens(&db, "worked");
    }
    db
}

/// The session `id` uses tokens.
fn use_tokens(db: &rusqlite::Connection, id: &str) {
    let now = Utc::now().timestamp();
    db.execute(
        "UPDATE sessions SET input_tokens = 100, output_tokens = 20 WHERE id = ?1",
        [id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage (session_id, model, input_tokens, output_tokens, first_seen, last_seen) VALUES (?1, 'claude-test', 100, 20, ?2, ?2)",
        rusqlite::params![id, now - 60],
    )
    .unwrap();
}

struct Fixture {
    _temp: tempfile::TempDir,
    /// Whether the sessions are this Mac's rather than the far machine's.
    here: bool,
    far: rusqlite::Connection,
    core: AgentHudCore,
}

fn fixture() -> Fixture {
    fixture_counted(true)
}

/// The same sessions in this Mac's own Hermes home.
fn here() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".hermes");
    let db = state_db(&home.join("state.db"), true);
    db.execute_batch("DROP TABLE hud_mirror;").unwrap();
    Fixture::open(temp, db, home, true)
}

/// With `counted` false, no session of the client has ever used a token.
fn fixture_counted(counted: bool) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let far_root = temp.path().join("hud/clients/hermes-far");
    let far = state_db(
        &far_root.join(format!("profiles/{KEY}-default/state.db")),
        counted,
    );
    Fixture::open(temp, far, far_root, false)
}

impl Fixture {
    fn open(temp: tempfile::TempDir, far: rusqlite::Connection, root: PathBuf, here: bool) -> Self {
        let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
        core.provider_roots = vec![(root, "hermes")];
        core.settings["sound"] = json!(true);
        let mut fixture = Fixture {
            _temp: temp,
            here,
            far,
            core,
        };
        fixture.refresh();
        fixture
    }
    /// The id the HUD gives the session `name`.
    fn id(&self, name: &str) -> String {
        if self.here {
            format!("hermes:{name}")
        } else {
            far(name)
        }
    }
    /// One whole collection, then the snapshot.
    fn refresh(&mut self) -> Value {
        let mut snapshot = self.core.request("refresh", json!({})).unwrap();
        for _ in 0..10 {
            if self.core.reads.pending.is_empty() {
                break;
            }
            snapshot = self.core.request("snapshot", json!({})).unwrap();
        }
        snapshot
    }
    /// The panel's short list (`leaveOutUnused`) or the Sessions page's:
    /// its session ids, sorted, and its total.
    fn list(&mut self, panel: bool) -> (Vec<String>, Value) {
        let mut params = json!({"scope": "active", "limit": 5});
        if panel {
            params["leaveOutUnused"] = json!(true);
        }
        let found = self.core.request("sessions", params).unwrap();
        let ids = found["items"].as_array().unwrap().iter();
        let ids = ids.map(|item| item["id"].as_str().unwrap().to_owned());
        let mut ids = ids.collect::<Vec<_>>();
        ids.sort();
        (ids, found["total"].clone())
    }
    /// A hook fact about `id`'s turn `turn`, as Hermes's channel tells one.
    fn hook(&mut self, id: &str, kind: &str, turn: &str, state: Option<&str>) {
        let at = Utc::now().timestamp_millis();
        self.core.ingest_hook_events(vec![HookEvent {
            source: "hermes".into(),
            session_id: self.id(id),
            kind: kind.into(),
            at_ms: at,
            data: json!({"state": state, "turnId": turn, "startedAtMs": at - 100, "outcome": null}),
        }]);
    }
    /// The `finished` alerts in a snapshot, by session.
    fn finished(&mut self) -> Vec<String> {
        alerts(&mut self.core, "finished")
    }
}

/// The alerts of `kind` a snapshot holds, by session.
fn alerts(core: &mut AgentHudCore, kind: &str) -> Vec<String> {
    std::thread::sleep(std::time::Duration::from_millis(5));
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap().iter();
    let alerts = alerts.filter(|alert| alert["kind"] == kind);
    alerts
        .map(|alert| alert["sessionId"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_session_that_used_no_ai_stays_out_of_the_short_list_and_on_the_sessions_page() {
    let mut fixture = fixture();
    // The Sessions page keeps it.
    let (all, total) = fixture.list(false);
    assert_eq!(all, [far("job"), far("worked")]);
    assert_eq!(total, 2);
    // The panel's short list leaves it out, and its "+N more" counts only
    // what it could show.
    let (short, total) = fixture.list(true);
    assert_eq!(short, [far("worked")]);
    assert_eq!(total, 1);
    // The snapshot's row says so, for the panel's first paint.
    let snapshot = fixture.core.request("snapshot", json!({})).unwrap();
    let row = |id: String| {
        let rows = snapshot["sessions"].as_array().unwrap().iter();
        rows.clone().find(|row| row["id"] == id).unwrap().clone()
    };
    assert_eq!(row(far("job"))["usedNoAi"], true);
    assert!(row(far("worked")).get("usedNoAi").is_none());
}

#[test]
fn a_session_that_used_no_ai_shows_while_it_runs_or_waits() {
    let mut fixture = fixture();
    for state in ["running", "waiting"] {
        fixture.hook("job", "session-observation", "t1", Some(state));
        let (short, total) = fixture.list(true);
        assert_eq!(short, [far("job"), far("worked")], "{state}");
        assert_eq!(total, 2, "{state}");
        let snapshot = fixture.core.request("snapshot", json!({})).unwrap();
        let rows = snapshot["sessions"].as_array().unwrap().iter();
        assert!(
            rows.clone().all(|row| row.get("usedNoAi").is_none()),
            "{state}"
        );
    }
}

#[test]
fn a_far_turn_that_used_no_ai_still_sounds_finished() {
    // A far machine's tokens come after its turn's end (its gateway's live
    // count, its history), so a zero then is no proof: its turn sounds.
    let mut fixture = fixture();
    let (job, worked) = (fixture.id("job"), fixture.id("worked"));
    for id in ["job", "worked"] {
        fixture.hook(id, "session-observation", "t1", Some("running"));
    }
    for id in ["job", "worked"] {
        fixture.hook(id, "completion", "t1", None);
    }
    let mut sounded = fixture.finished();
    sounded.sort();
    assert_eq!(sounded, [job.clone(), worked.clone()]);
    // It is still left out of the short list while it shows no tokens.
    assert_eq!(fixture.list(true).0, [worked]);
}

#[test]
fn a_turn_here_that_used_no_ai_sounds_no_finished_though_its_session_uses_tokens_later() {
    let mut fixture = here();
    let job = fixture.id("job");
    assert_eq!(fixture.list(false).0, [job, fixture.id("worked")]);
    assert_eq!(fixture.list(true).0, [fixture.id("worked")]);
    turn_stays_silent(fixture);
}

fn turn_stays_silent(mut fixture: Fixture) {
    let (job, worked) = (fixture.id("job"), fixture.id("worked"));
    for id in ["job", "worked"] {
        fixture.hook(id, "session-observation", "t1", Some("running"));
    }
    assert!(fixture.finished().is_empty());
    for id in ["job", "worked"] {
        fixture.hook(id, "completion", "t1", None);
    }
    // The turn that used tokens sounds; the one that used none does not.
    assert_eq!(fixture.finished(), std::slice::from_ref(&worked));
    assert_eq!(
        fixture.finished(),
        std::slice::from_ref(&worked),
        "nothing new"
    );
    // Tokens it uses later bring it back to the short list; the turn that
    // ended with none stays silent, however its row reads now.
    use_tokens(&fixture.far, "job");
    fixture.refresh();
    let (short, _) = fixture.list(true);
    assert_eq!(short, [job, worked.clone()]);
    assert_eq!(fixture.finished(), [worked]);
}

#[test]
fn a_silent_turn_stays_silent_when_its_session_waits_again() {
    let mut fixture = here();
    let job = fixture.id("job");
    fixture.hook("job", "session-observation", "t1", Some("running"));
    fixture.hook("job", "completion", "t1", None);
    assert!(fixture.finished().is_empty());
    // The next turn waits on the person: it sounds `needs-you`, and the
    // first turn's end is not weighed again against the waiting row.
    std::thread::sleep(std::time::Duration::from_millis(200));
    fixture.hook("job", "session-observation", "t2", Some("waiting"));
    assert_eq!(alerts(&mut fixture.core, "needs-you"), [job]);
    assert!(fixture.finished().is_empty());
}

#[test]
fn every_unused_session_left_out_is_counted_for_the_panel() {
    // Hermes records every model call's tokens, so its zeros mean none, even
    // where no session here ever called a model.
    let mut fixture = fixture_counted(false);
    assert_eq!(fixture.list(false).0, [far("job"), far("worked")]);
    let params = json!({"scope": "active", "limit": 5, "leaveOutUnused": true});
    let found = fixture.core.request("sessions", params).unwrap();
    assert_eq!(found["items"], json!([]));
    assert_eq!(found["total"], 0);
    // What the panel needs to lead to them rather than say there are none.
    assert_eq!(found["leftOut"], 2);
    let page = json!({"scope": "active", "limit": 5});
    let found = fixture.core.request("sessions", page).unwrap();
    assert_eq!(found["leftOut"], 0);
}

/// Qoder writes a credits-only reply's tokens as zeros (`qoder/transcript.rs`):
/// a session of such replies did use a model, beside one that counted tokens.
fn qoder(home: &Path, id: &str, tokens: u64) {
    let at = Utc::now() - chrono::Duration::seconds(30);
    let usage = if tokens == 0 {
        json!({"input_tokens": 0, "output_tokens": 0, "credits": 1.5})
    } else {
        json!({"input_tokens": tokens, "output_tokens": 20})
    };
    let lines = [
        json!({"sessionId": id, "timestamp": at.to_rfc3339(), "type": "user",
            "message": {"role": "user", "content": "Synthetic prompt"}}),
        json!({"sessionId": id, "timestamp": at.to_rfc3339(), "type": "assistant",
            "message": {"id": format!("m-{id}"), "role": "assistant", "model": "auto",
                "content": [{"type": "text", "text": "Synthetic reply"}], "usage": usage}}),
    ];
    let path = home
        .join(".qoder/projects/work")
        .join(format!("{id}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    fs::write(path, text.join("\n") + "\n").unwrap();
}

#[test]
fn a_client_that_can_reply_without_tokens_never_hides_a_session_or_loses_a_sound() {
    let temp = tempfile::tempdir().unwrap();
    let (credits, counted) = (
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    );
    qoder(temp.path(), credits, 0);
    qoder(temp.path(), counted, 120);
    let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
    core.provider_roots = vec![(temp.path().join(".qoder/projects"), "qoder")];
    core.settings["sound"] = json!(true);
    core.request("refresh", json!({})).unwrap();
    let params = json!({"scope": "active", "limit": 5, "leaveOutUnused": true});
    let found = core.request("sessions", params).unwrap();
    let mut ids: Vec<String> = (found["items"].as_array().unwrap().iter())
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    assert_eq!(ids.len(), 2, "{found}");
    assert_eq!(found["leftOut"], 0);
    let rows = core.request("snapshot", json!({})).unwrap()["sessions"].clone();
    assert!((rows.as_array().unwrap().iter()).all(|row| row.get("usedNoAi").is_none()));
    for id in &ids {
        let at = Utc::now().timestamp_millis();
        core.ingest_hook_events(vec![HookEvent {
            source: "qoder".into(),
            session_id: id.clone(),
            kind: "completion".into(),
            at_ms: at,
            data: json!({"turnId": "t1", "startedAtMs": at - 100}),
        }]);
    }
    let mut sounded = alerts(&mut core, "finished");
    sounded.sort();
    assert_eq!(sounded, ids, "the credits-only reply's turn sounds too");
}
