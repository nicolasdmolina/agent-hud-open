//! A far session's count while it runs (`Record::live_total`): shown on
//! its row, and never counted. The ledger adds running totals by record
//! key and never takes one back, so a count read from a far gateway may
//! not enter it: the far database's rows would later be added on top.
//! These tests write a mirror the way its writer does (`mirror::claim`,
//! `list`, `live`) and put the far database's rows in by hand, as the
//! history reader will, in every order the two can arrive in.
use super::*;
use agent_hud_agents::hermes::{
    gateway::{machine_key, Listed, Profile, Sample},
    mirror as written,
};

/// What the made-up install id makes at the made-up far root home.
const KEY: &str = "aaeda597d860920c";

/// A far machine's mirror with one session, `s1`, from the live list.
struct Far {
    temp: tempfile::TempDir,
    db: rusqlite::Connection,
    /// When the fixture was made, in seconds.
    now: i64,
}

fn far() -> Far {
    let temp = tempfile::tempdir().unwrap();
    let key = machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap();
    assert_eq!(key.as_str(), KEY);
    let root = temp.path().join("hud").join(written::ROOT);
    let path = written::file(&root, &key, &Profile::Default);
    fs::create_dir_all(path.parent().unwrap()).unwrap(); // plain directory
    let db = rusqlite::Connection::open(&path).unwrap();
    assert!(written::claim(&db, &key, &Profile::Default, "Mac mini").unwrap());
    let now = Utc::now().timestamp();
    let far = Far { temp, db, now };
    far.lists("s1");
    far
}

impl Far {
    fn open(&self) -> AgentHudCore {
        let mut core = AgentHudCore::new(vec![], self.temp.path().join("app"));
        let root = self.temp.path().join("hud").join(written::ROOT);
        core.provider_roots = vec![(root, "hermes")];
        core
    }

    /// A poll lists `session` as live.
    fn lists(&self, session: &str) {
        let said = Listed {
            title: Some("Far turn".into()),
            model: Some("claude-test".into()),
            source: Some("telegram".into()),
            started_at: Some((self.now - 30) as f64),
        };
        let listed = [(session.to_owned(), said)];
        assert_eq!(
            written::list(&self.db, &listed, self.now * 1000).unwrap(),
            1
        );
    }

    /// A sample of `total` tokens for `session`, read `at` seconds after
    /// the fixture was made; how many rows the writer's write changed.
    fn sample(&self, session: &str, total: u64, at: i64) -> usize {
        let sample = Sample {
            input: total * 7 / 10,
            output: total / 5,
            cache_read: total * 8 / 100,
            cache_write: total - total * 7 / 10 - total / 5 - total * 8 / 100,
        };
        assert_eq!(sample.total(), total);
        let read_ms = (self.now + at) * 1000;
        written::live(&self.db, &[(session.to_owned(), sample, read_ms)]).unwrap()
    }

    /// The far database's rows for `session`, read `at` seconds after
    /// the fixture was made: one usage row for each model and provider
    /// with its tokens, and the session's own row, as one history batch
    /// writes them. A batch read no later than the one the mirror holds
    /// for the session is dropped, unless `again` writes it all the same.
    fn history(&self, session: &str, usage: &[(&str, &str, i64)], at: i64, again: bool) {
        let read_ms = (self.now + at) * 1000;
        let held: i64 = (self.db)
            .query_row(
                "SELECT read_ms FROM hud_rows WHERE session_id=?1 AND durable=1",
                [session],
                |row| row.get(0),
            )
            .unwrap_or(i64::MIN);
        if read_ms <= held && !again {
            return;
        }
        let total: i64 = usage.iter().map(|row| row.2).sum();
        self.db
            .execute(
                "UPDATE sessions SET input_tokens=?2, output_tokens=0, title='Far turn, stored',
                    last_activity_at=?3 WHERE id=?1",
                rusqlite::params![session, total, self.now - 5],
            )
            .unwrap();
        self.db
            .execute(
                "DELETE FROM session_model_usage WHERE session_id=?1",
                [session],
            )
            .unwrap();
        for (model, provider, tokens) in usage {
            self.db
                .execute(
                    "INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens,
                        output_tokens, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
                    rusqlite::params![session, model, provider, tokens, self.now - 20],
                )
                .unwrap();
        }
        self.db
            .execute(
                "UPDATE hud_rows SET durable=1, read_ms=?2, print='print' WHERE session_id=?1",
                rusqlite::params![session, read_ms.max(held)],
            )
            .unwrap();
    }

    /// What `hud_live` holds, as text.
    fn live(&self) -> String {
        let sql = "SELECT session_id, input_tokens, output_tokens, cache_read_tokens,
            cache_write_tokens, read_ms FROM hud_live ORDER BY session_id";
        let mut statement = self.db.prepare(sql).unwrap();
        let rows = statement.query_map([], |row| {
            let counts: [i64; 5] = [
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ];
            Ok(format!("{} {counts:?}", row.get::<_, String>(0)?))
        });
        rows.unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Two models through two providers: 60 and 40.
const TWO: [(&str, &str, i64); 2] = [("claude-test", "anthropic", 60), ("gpt-test", "openai", 40)];

/// One whole collection.
fn refresh(core: &mut AgentHudCore) {
    core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        if core.reads.pending.is_empty() {
            break;
        }
        core.request("snapshot", json!({})).unwrap();
    }
}

/// The ledger itself: how many Hermes events it holds, and their tokens.
fn ledger(core: &AgentHudCore) -> (i64, i64) {
    let sql = "SELECT COUNT(*), TOTAL(input+output+cache_read+cache_write) FROM usage_meter_event WHERE source='hermes'";
    let read = |row: &rusqlite::Row| Ok((row.get(0)?, row.get::<_, f64>(1)? as i64));
    let db = core.database.as_ref().unwrap();
    db.query_row(sql, [], read).unwrap()
}

/// What the session list shows of `session`: its tokens.
fn shown(core: &mut AgentHudCore, session: &str) -> Value {
    let found = core
        .request("sessions", json!({"scope": "all", "search": ""}))
        .unwrap();
    let id = format!("hermes:{KEY}/default:{session}");
    let items = found["items"].as_array().unwrap();
    let row = items.iter().find(|item| item["id"] == id.as_str());
    row.unwrap_or_else(|| panic!("no row for {id}: {found}"))["tokens"].clone()
}

/// The same from the snapshot, which the HUD's own list is drawn from,
/// and from the session's page.
fn shown_everywhere(core: &mut AgentHudCore, session: &str) -> [Value; 3] {
    let id = format!("hermes:{KEY}/default:{session}");
    let listed = shown(core, session);
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let rows = snapshot["sessions"].as_array().unwrap();
    let hud = rows.iter().find(|row| row["id"] == id.as_str());
    let hud = hud.map_or(Value::Null, |row| row["tokens"].clone());
    let detail = core
        .request("session_detail", json!({"id": id, "source": "hermes"}))
        .unwrap();
    [listed, hud, detail["session"]["tokens"].clone()]
}

/// A value with every clock taken out: when it was worked out, which
/// reading of the ledger it is (a count of how often the logs were
/// taken stock of, which every pass that read anything moves), up to
/// when the ledger has looked. None of them is usage.
fn timeless(mut value: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for clock in ["generatedAt", "dataRevision", "revision", "observedTo"] {
                    map.remove(clock);
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

/// Statistics over the month around the fixture, every kind of token.
fn statistics(core: &mut AgentHudCore, far: &Far) -> Value {
    let iso = |at: i64| DateTime::<Utc>::from_timestamp(at, 0).unwrap().to_rfc3339();
    let query = json!({"from": iso(far.now - 30 * 86_400), "to": iso(far.now + 86_400),
        "bucketMinutes": 1440, "costKinds": "all", "timeZone": "UTC",
        "tokenKinds": ["input", "output", "cacheRead", "cacheWrite", "reasoning"]});
    timeless(core.request("statistics", query).unwrap())
}

fn counted(core: &mut AgentHudCore, far: &Far) -> Value {
    statistics(core, far)["totals"]["selectedTokens"].clone()
}

/// The acceptance case: one session, two models through two providers.
/// Live samples totalling 100, then the far database's rows of 60 and
/// 40. The ledger holds 100, statistics show 100, the row shows 100.
#[test]
fn live_samples_then_durable_rows_count_once() {
    let far = far();
    let mut core = far.open();
    refresh(&mut core);
    assert_eq!(shown(&mut core, "s1"), json!(0));
    // The count rises while the turn runs, and nothing is counted.
    for (total, at) in [(30, 1), (70, 2), (100, 3)] {
        assert_eq!(far.sample("s1", total, at), 1);
        refresh(&mut core);
        assert_eq!(
            shown_everywhere(&mut core, "s1"),
            [json!(total), json!(total), json!(total)]
        );
        assert_eq!(ledger(&core), (0, 0), "after a sample of {total}");
        assert_eq!(counted(&mut core, &far), json!(0));
    }
    // The far database's rows arrive, under two other record keys.
    far.history("s1", &TWO, 10, false);
    refresh(&mut core);
    assert_eq!(ledger(&core), (2, 100));
    assert_eq!(counted(&mut core, &far), json!(100));
    assert_eq!(
        shown_everywhere(&mut core, "s1"),
        [json!(100), json!(100), json!(100)]
    );
}

#[test]
fn durable_rows_first_then_live_samples_count_once() {
    let far = far();
    let mut core = far.open();
    far.history("s1", &TWO, 1, false);
    refresh(&mut core);
    assert_eq!(ledger(&core), (2, 100));
    assert_eq!(shown(&mut core, "s1"), json!(100));
    // A sample that says the same, then one that says more: the turn
    // went on after the far database was read.
    for (total, row) in [(100, 100), (130, 130)] {
        assert_eq!(far.sample("s1", total, total as i64), 1);
        refresh(&mut core);
        assert_eq!(shown(&mut core, "s1"), json!(row));
        assert_eq!(ledger(&core), (2, 100));
        assert_eq!(counted(&mut core, &far), json!(100));
    }
}

#[test]
fn durable_rows_between_two_live_samples_count_once() {
    let far = far();
    let mut core = far.open();
    assert_eq!(far.sample("s1", 50, 1), 1);
    refresh(&mut core);
    assert_eq!((shown(&mut core, "s1"), ledger(&core)), (json!(50), (0, 0)));
    far.history("s1", &TWO, 2, false);
    refresh(&mut core);
    assert_eq!(
        (shown(&mut core, "s1"), ledger(&core)),
        (json!(100), (2, 100))
    );
    assert_eq!(far.sample("s1", 100, 3), 1);
    refresh(&mut core);
    assert_eq!(
        (shown(&mut core, "s1"), ledger(&core)),
        (json!(100), (2, 100))
    );
    assert_eq!(counted(&mut core, &far), json!(100));
}

/// The same with the far history's own write (`mirror::durable`, ticket
/// 37) in place of rows put in by hand: a live-list row, samples, then
/// the far database's rows, which grow later. Each token is counted
/// once, the write leaves `hud_live` as it was, and the row shows the
/// larger number throughout.
#[test]
fn the_history_write_and_live_samples_count_each_token_once() {
    use agent_hud_agents::hermes::far_history::{Cell, Session};
    let far = far();
    let mut core = far.open();
    let text = |text: &str| Cell::Text(text.into());
    let at = |ago: i64| Cell::Real((far.now - ago) as f64);
    // `s1` as the far database holds it: one usage row for each model
    // and provider, of these input tokens and 10 output tokens.
    let stored = |usage: &[(&str, &str, i64)], print: &str| {
        let input: i64 = usage.iter().map(|row| row.2).sum();
        let output = 10 * usage.len() as i64;
        let row = |(model, provider, input): &(&str, &str, i64)| {
            let named = [text(model), text(provider), text(""), text("api"), text("")];
            let counts = [Cell::Int(*input), Cell::Int(10), Cell::Int(0), Cell::Int(0)];
            let cells = named.into_iter().chain(counts).chain([at(20), at(5)]);
            cells.collect::<Vec<_>>()
        };
        Session {
            rowid: 1,
            id: "s1".to_owned(),
            print: print.to_owned(),
            cells: vec![
                at(30),
                text("Far turn, stored"),
                Cell::Null,
                text("telegram"),
                text("claude-test"),
                text("anthropic"),
                text(""),
                text("api"),
                Cell::Int(input),
                Cell::Int(output),
                Cell::Int(0),
                Cell::Int(0),
                Cell::Null,
                at(5),
                Cell::Null,
            ],
            usage: usage.iter().map(row).collect(),
        }
    };
    let write = |session: Session, read_at: i64| {
        let live = far.live();
        let read_ms = (far.now + read_at) * 1000;
        let written = written::durable(&far.db, &[session], read_ms).unwrap();
        assert_eq!(far.live(), live, "the history write touched hud_live");
        written
    };

    // The turn runs: the count on the row follows, nothing is counted.
    for (total, at) in [(40, 1), (100, 2)] {
        assert_eq!(far.sample("s1", total, at), 1);
        refresh(&mut core);
        assert_eq!(shown(&mut core, "s1"), json!(total));
        assert_eq!(ledger(&core), (0, 0));
    }
    // The far database is read: 60 and 40, through two providers.
    let first = [("claude-test", "anthropic", 50), ("gpt-test", "openai", 30)];
    assert_eq!(write(stored(&first, "print-1"), 10), 1);
    refresh(&mut core);
    assert_eq!(ledger(&core), (2, 100));
    assert_eq!(counted(&mut core, &far), json!(100));
    assert_eq!(
        shown_everywhere(&mut core, "s1"),
        [json!(100), json!(100), json!(100)]
    );
    // The turn goes on, and the gateway says more than the far database
    // did when it was read.
    assert_eq!(far.sample("s1", 130, 11), 1);
    refresh(&mut core);
    assert_eq!((shown(&mut core, "s1"), ledger(&core).1), (json!(130), 100));
    // The same batch again, and one read earlier: neither is written.
    assert_eq!(write(stored(&first, "print-1"), 10), 0);
    assert_eq!(
        write(stored(&[("claude-test", "anthropic", 5)], "print-0"), 9),
        0
    );
    refresh(&mut core);
    assert_eq!((shown(&mut core, "s1"), ledger(&core).1), (json!(130), 100));
    // The far database is read again, grown to 90 and 70: the ledger
    // counts the growth and no token twice, and passes the live count.
    let grown = [("claude-test", "anthropic", 80), ("gpt-test", "openai", 60)];
    assert_eq!(write(stored(&grown, "print-2"), 20), 1);
    refresh(&mut core);
    assert_eq!(ledger(&core).1, 160);
    assert_eq!(counted(&mut core, &far), json!(160));
    assert_eq!(
        shown_everywhere(&mut core, "s1"),
        [json!(160), json!(160), json!(160)]
    );
    // A last sample that says what the gateway counted, which is less.
    assert_eq!(far.sample("s1", 150, 21), 1);
    refresh(&mut core);
    assert_eq!((shown(&mut core, "s1"), ledger(&core).1), (json!(160), 160));
    // And after a relaunch.
    drop(core);
    let mut core = far.open();
    refresh(&mut core);
    assert_eq!((shown(&mut core, "s1"), ledger(&core).1), (json!(160), 160));
    assert_eq!(counted(&mut core, &far), json!(160));
}

/// A history batch read before a live sample and written after it:
/// `hud_live` is as it was, and the row still shows the larger number.
#[test]
fn a_history_batch_written_after_a_newer_sample_leaves_the_sample_alone() {
    let far = far();
    let mut core = far.open();
    assert_eq!(far.sample("s1", 130, 9), 1);
    refresh(&mut core);
    let live = far.live();
    // Read at second 4, when the far database held 100; written now.
    far.history("s1", &TWO, 4, false);
    assert_eq!(far.live(), live);
    refresh(&mut core);
    assert_eq!(far.live(), live);
    assert_eq!(shown(&mut core, "s1"), json!(130));
    assert_eq!(ledger(&core), (2, 100));
    assert_eq!(counted(&mut core, &far), json!(100));
}

/// The same live sample written twice, and the same history batch
/// written twice: nothing changes the second time.
#[test]
fn the_same_sample_and_the_same_batch_twice_change_nothing() {
    let far = far();
    let mut core = far.open();
    assert_eq!(far.sample("s1", 120, 5), 1);
    far.history("s1", &TWO, 6, false);
    refresh(&mut core);
    let all = |core: &mut AgentHudCore, far: &Far| {
        let rows = shown_everywhere(core, "s1");
        (
            rows,
            ledger(core),
            statistics(core, far).to_string(),
            far.live(),
        )
    };
    let before = all(&mut core, &far);
    assert_eq!(before.0, [json!(120), json!(120), json!(120)]);
    assert_eq!(before.1, (2, 100));

    assert_eq!(far.sample("s1", 120, 5), 0, "the same sample");
    refresh(&mut core);
    assert_eq!(all(&mut core, &far), before);
    // The batch again: dropped as no newer, and then written all the
    // same, every row of it as it was.
    far.history("s1", &TWO, 6, false);
    refresh(&mut core);
    assert_eq!(all(&mut core, &far), before);
    far.history("s1", &TWO, 6, true);
    refresh(&mut core);
    assert_eq!(all(&mut core, &far), before);
    // Nor after a relaunch, which reads the mirror from what was saved.
    drop(core);
    let mut core = far.open();
    refresh(&mut core);
    assert_eq!(all(&mut core, &far), before);
}

/// A zero sample after a non-zero one, and a lower one after a higher
/// one, leave the count where it was. So does a `hud_live` that says
/// less, or nothing, however it came to: what is shown never goes down.
#[test]
fn a_zero_or_lower_sample_leaves_the_count_where_it_was() {
    let far = far();
    let mut core = far.open();
    assert_eq!(far.sample("s1", 100, 1), 1);
    refresh(&mut core);
    assert_eq!(shown(&mut core, "s1"), json!(100));
    for (lower, at) in [(0, 2), (40, 3), (100, 4)] {
        assert_eq!(far.sample("s1", lower, at), 0, "a sample of {lower}");
        refresh(&mut core);
        assert_eq!(shown(&mut core, "s1"), json!(100), "after {lower}");
    }
    for tampered in [
        "UPDATE hud_live SET input_tokens=1, output_tokens=0, cache_read_tokens=0, cache_write_tokens=0",
        "DELETE FROM hud_live",
    ] {
        far.db.execute(tampered, []).unwrap();
        refresh(&mut core);
        assert_eq!(shown(&mut core, "s1"), json!(100), "{tampered}");
    }
    assert_eq!(ledger(&core), (0, 0));
}

/// After a relaunch with the far machine out of reach, so that nothing
/// writes the mirror, the row shows the same count: from the saved head
/// before anything is read, and after the mirror is read again.
#[test]
fn the_count_survives_a_relaunch_with_the_machine_offline() {
    let far = far();
    let mut core = far.open();
    assert_eq!(far.sample("s1", 100, 1), 1);
    refresh(&mut core);
    assert_eq!(shown(&mut core, "s1"), json!(100));
    drop(core);

    let mut core = far.open();
    let head = core.logs.values().find(|head| head.id.ends_with(":s1"));
    assert_eq!(head.expect("the saved head").live_total, Some(100));
    assert_eq!(
        shown(&mut core, "s1"),
        json!(100),
        "before anything is read"
    );
    refresh(&mut core);
    assert_eq!(
        shown_everywhere(&mut core, "s1"),
        [json!(100), json!(100), json!(100)]
    );
    assert_eq!(ledger(&core), (0, 0));
}

/// A count that moved is news for whoever shows the rows: the snapshot's
/// count of the sessions' changes moves with it, and stands still for a
/// sample that says the same.
#[test]
fn a_new_count_moves_the_sessions_revision_and_the_same_one_does_not() {
    let far = far();
    let mut core = far.open();
    let revision = |core: &mut AgentHudCore| {
        refresh(core);
        // Past the second the list's fingerprint is held for.
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        core.request("snapshot", json!({})).unwrap()["sessionsRevision"].clone()
    };
    let first = revision(&mut core);
    assert_eq!(revision(&mut core), first, "nothing changed");
    assert_eq!(far.sample("s1", 100, 1), 1);
    let second = revision(&mut core);
    assert_ne!(second, first, "a first count");
    assert_eq!(far.sample("s1", 100, 2), 0);
    assert_eq!(revision(&mut core), second, "the same count");
    assert_eq!(far.sample("s1", 250, 3), 1);
    assert_ne!(revision(&mut core), second, "a count that grew");
}

/// Every row of the ledger's usage and quota tables, as text.
fn ledger_tables(core: &AgentHudCore) -> String {
    let db = core.database.as_ref().unwrap();
    let mut names = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let names: Vec<String> = (names.query_map([], |row| row.get(0)).unwrap())
        .map(Result::unwrap)
        .collect();
    assert!(names.iter().any(|name| name == "usage_meter_event"));
    assert!(names.iter().any(|name| name == "usage_meter_bucket"));
    let mut all = String::new();
    // The tables that hold usage and quota. The others say where each
    // session's log is and hold its saved head, with when each was
    // written: the head is where the count to show is kept.
    let metered = |name: &&String| name.starts_with("usage_meter_") || name.starts_with("quota_");
    for table in names.iter().filter(metered) {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let width = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let cells: Vec<rusqlite::types::Value> =
                (0..width).map(|at| row.get(at).unwrap()).collect();
            all += &format!("{table} {cells:?}\n");
        }
    }
    all
}

/// Statistics, cost and quota for the source are the same bytes with and
/// without `hud_live` rows, and so is every row of the ledger's usage
/// and quota tables.
#[test]
fn statistics_cost_and_quota_are_the_same_with_and_without_live_samples() {
    let far = far();
    far.lists("s2");
    far.history("s1", &TWO, 1, false);
    let mut core = far.open();
    refresh(&mut core);
    let iso = |at: i64| DateTime::<Utc>::from_timestamp(at, 0).unwrap().to_rfc3339();
    let (from, to) = (iso(far.now - 86_400), iso(far.now + 3_600));
    let answers = |core: &mut AgentHudCore, far: &Far| {
        let asked = [
            ("usage_buckets", json!({"from": from, "to": to})),
            ("quota_history", json!({"from": from, "to": to})),
            ("plan_value", json!({})),
            ("price_list", json!({})),
        ];
        let mut all = vec![statistics(core, far).to_string()];
        for (request, params) in asked {
            let mut answer = timeless(core.request(request, params).unwrap());
            if request == "plan_value" {
                // Its month ends at the moment it is asked.
                let whole = answer.as_object_mut().unwrap();
                assert!(whole.remove("from").is_some() && whole.remove("to").is_some());
            }
            all.push(answer.to_string());
        }
        // What the snapshot totals, which the HUD shows above its list.
        let snapshot = core.request("snapshot", json!({})).unwrap();
        all.push(snapshot["totals"].to_string());
        all.push(ledger_tables(core));
        all
    };
    let without = answers(&mut core, &far);
    assert!(far.live().is_empty());
    assert!(
        without[0].contains("\"selectedTokens\":100"),
        "{}",
        without[0]
    );

    // A sample for the session the far database gave, and for the one
    // that has only its row from the live list.
    assert_eq!(far.sample("s1", 500, 5), 1);
    assert_eq!(far.sample("s2", 900, 5), 1);
    refresh(&mut core);
    assert_eq!(shown(&mut core, "s1"), json!(500));
    assert_eq!(shown(&mut core, "s2"), json!(900));
    let with = answers(&mut core, &far);
    for (at, (with, without)) in with.iter().zip(&without).enumerate() {
        assert_eq!(with, without, "answer {at}");
    }
    // And after a relaunch.
    drop(core);
    let mut core = far.open();
    refresh(&mut core);
    assert_eq!(answers(&mut core, &far), without);
    assert_eq!(shown(&mut core, "s2"), json!(900));
}

/// A session of any other client has no such count: its row is the
/// ledger's total, and neither its saved head nor its saved log names
/// one.
#[test]
fn a_session_with_no_live_total_shows_the_ledgers_total_and_saves_no_field() {
    let far = far();
    far.history("s1", &TWO, 1, false);
    let mut core = far.open();
    refresh(&mut core);
    assert_eq!(shown(&mut core, "s1"), json!(100));
    let head = core.logs.values().find(|head| head.id.ends_with(":s1"));
    assert_eq!(head.unwrap().live_total, None);
    let db = core.database.as_ref().unwrap();
    let mut saved = db.prepare("SELECT * FROM ingestion_logs").unwrap();
    let width = saved.column_count();
    let mut rows = saved.query([]).unwrap();
    let mut seen = 0;
    while let Some(row) = rows.next().unwrap() {
        for at in 0..width {
            if let Ok(text) = row.get::<_, String>(at) {
                assert!(!text.contains("live_total"), "{text}");
                seen += 1;
            }
        }
    }
    assert!(seen > 0);
}
