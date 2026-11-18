//! The mirror writer with a real core behind it and no file watcher at
//! all: core is told that nothing changed before every pass, as it is
//! by a watcher that has nothing to report, so whatever is read was read
//! because the writer said which file it wrote (`hooks::Changed`). A
//! turn here is what the runtime does at the turn a channel's wake asks
//! for (`agent_hud_runtime`, `request`): the paths told are handed to
//! `written_files`, and a pass reads them. Every machine, label and
//! session is made up.
use super::*;
use agent_hud_agents::hermes::gateway::machine_key;
use agent_hud_core::{AgentHudCore, SourceRoot};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant, SystemTime},
};

mod failed;
mod history;
mod large;

/// How soon after its commit a write must be listed (ticket 34).
const SOON: Duration = Duration::from_secs(2);

/// The machine's route number, which a test raises; and a count of the
/// times it was asked, so a test can raise it between the writer's look
/// and its commit.
#[derive(Default)]
struct Number {
    now: AtomicU64,
    asked: AtomicUsize,
    /// Raises the number when it is asked for this time; 0 for never.
    raise_at: AtomicUsize,
}

impl Routes for Number {
    fn under(&self, number: u64, commit: &mut dyn FnMut()) -> bool {
        let asked = self.asked.fetch_add(1, Ordering::AcqRel) + 1;
        if asked == self.raise_at.load(Ordering::Acquire) {
            self.now.fetch_add(1, Ordering::AcqRel);
        }
        let current = self.now.load(Ordering::Acquire) == number;
        if current {
            commit();
        }
        current
    }
}

/// What the writer told, as the hooks keep it: the paths, and when each
/// word came.
#[derive(Default)]
struct Told {
    paths: Mutex<HashSet<PathBuf>>,
    at: Mutex<Vec<Instant>>,
}

struct Bench {
    data: PathBuf,
    core: AgentHudCore,
    told: Arc<Told>,
    number: Arc<Number>,
    key: MachineKey,
    writer: Arc<Writer>,
    _temp: tempfile::TempDir,
}

fn key() -> MachineKey {
    machine_key(Some("0123456789abcdef0123456789abcdef"), "/far/.hermes").unwrap()
}

fn reports() -> Profile {
    Profile::named("reports").unwrap()
}

fn bench(label: &str) -> Bench {
    bench_spaced(label, super::SPACE)
}

/// A bench whose writer commits counts no more often than `space`.
fn bench_spaced(label: &str, space: Duration) -> Bench {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().canonicalize().unwrap().join("data");
    crate::os::private_directory(&data).unwrap();
    let roots = vec![
        SourceRoot {
            path: data.join(mirror::ROOT),
            provider: "hermes",
        },
        SourceRoot {
            path: data.join("claude"),
            provider: "claude",
        },
    ];
    let mut core = AgentHudCore::new(roots, data.clone());
    // The first pass walks every root, as at launch: nothing is there.
    core.request("refresh", json!({})).unwrap();
    let told = Arc::new(Told::default());
    let number = Arc::new(Number::default());
    number.now.store(1, Ordering::Release);
    let writer = writer_spaced(&data, label, &told, &number, space);
    Bench {
        data,
        core,
        told,
        number,
        key: key(),
        writer,
        _temp: temp,
    }
}

fn writer(data: &Path, label: &str, told: &Arc<Told>, number: &Arc<Number>) -> Arc<Writer> {
    writer_spaced(data, label, told, number, super::SPACE)
}

fn writer_spaced(
    data: &Path,
    label: &str,
    told: &Arc<Told>,
    number: &Arc<Number>,
    space: Duration,
) -> Arc<Writer> {
    let kept = told.clone();
    let changed: Changed = Arc::new(move |paths: &[PathBuf]| {
        kept.paths.lock().unwrap().extend(paths.iter().cloned());
        kept.at.lock().unwrap().push(Instant::now());
    });
    let log = Log::new(data).of(Some("mirror"));
    let writer = Writer::spaced(data, (&key(), label), number.clone(), changed, log, space);
    writer.unwrap()
}

fn said(title: &str) -> Listed {
    Listed {
        title: Some(title.into()),
        model: Some("claude-test".into()),
        source: Some("telegram".into()),
        started_at: Some(1_760_000_000.0),
    }
}

/// A poll's listing of `profile`: these sessions, read at `read_ms`.
fn listing(profile: &Profile, sessions: &[(&str, &str)], read_ms: i64) -> Listing {
    let rows = sessions
        .iter()
        .map(|(id, title)| ((*id).to_owned(), said(title)))
        .collect();
    Listing {
        read_ms,
        profiles: BTreeMap::from([(profile.clone(), rows)]),
    }
}

impl Bench {
    /// Queues a listing read under the route number `route` and waits
    /// until the writer has dealt with it.
    fn lists(&self, profile: &Profile, sessions: &[(&str, &str)], read_ms: i64, route: u64) {
        self.writer
            .listed(listing(profile, sessions, read_ms), route);
        self.writer.settle();
    }

    /// How often the writer told core so far.
    fn tells(&self) -> usize {
        self.told.at.lock().unwrap().len()
    }

    /// A pass with the file watcher silent and nothing told: core hears
    /// that no file changed.
    fn silent_pass(&mut self) {
        self.core.file_changes(Some(HashSet::new()));
        self.core.request("snapshot", json!({})).unwrap();
    }

    /// The turn the writer's word asks for: what it told goes to core,
    /// and a pass reads it. How many paths that were.
    fn turn(&mut self) -> usize {
        let told = std::mem::take(&mut *self.told.paths.lock().unwrap());
        let count = told.len();
        self.core.written_files(told);
        self.core.request("snapshot", json!({})).unwrap();
        count
    }

    /// Each far session listed: its id after the machine's key, the
    /// machine it shows, its title and its tokens.
    fn rows(&mut self) -> Vec<(String, Value, Value, Value)> {
        let query = json!({"scope": "all", "search": ""});
        let found = self.core.request("sessions", query).unwrap();
        let far = format!("hermes:{}/", self.key.as_str());
        let mut rows: Vec<_> = (found["items"].as_array().unwrap().iter())
            .filter_map(|item| {
                let id = item["id"].as_str()?.strip_prefix(&far)?.to_owned();
                let shown = |key: &str| item[key].clone();
                Some((id, shown("machine"), shown("title"), shown("tokens")))
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    fn ids(&mut self) -> Vec<String> {
        self.rows().into_iter().map(|row| row.0).collect()
    }

    fn file(&self, profile: &Profile) -> PathBuf {
        self.writer.file(profile)
    }

    /// Every row of the mirror of `profile`, as text.
    fn dump(&self, profile: &Profile) -> String {
        // Read-only, so that looking at a mirror never changes its file.
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let db = rusqlite::Connection::open_with_flags(self.file(profile), flags).unwrap();
        let mut all = String::new();
        for table in ["sessions", "session_model_usage", "hud_mirror", "hud_rows"] {
            let sql = format!("SELECT * FROM {table} ORDER BY 1, 2");
            let mut statement = db.prepare(&sql).unwrap();
            let width = statement.column_count();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let cells: Vec<rusqlite::types::Value> =
                    (0..width).map(|n| row.get(n).unwrap()).collect();
                all += &format!("{table} {cells:?}\n");
            }
        }
        all
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.data.join("hermes-gateway.log")).unwrap_or_default()
    }
}

/// The main file's size and time of change, and the write-ahead log's.
mod live;

fn stamps(path: &Path) -> [(u64, SystemTime); 2] {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    [path, wal.as_path()].map(|path| {
        let meta = std::fs::metadata(path).unwrap();
        (meta.len(), meta.modified().unwrap())
    })
}

#[test]
fn the_first_mirror_is_read_without_file_events() {
    let mut bench = bench("Mac mini");
    assert!(bench.rows().is_empty());
    assert!(!bench.data.join(mirror::ROOT).exists(), "no mirror yet");

    bench.lists(&Profile::Default, &[("s1", "Plan the week")], 1_000, 1);
    let committed = bench.told.at.lock().unwrap()[0];
    assert!(bench.file(&Profile::Default).is_file());
    // Nothing reads outside a pass: the writer only said which file.
    assert!(bench.rows().is_empty());
    assert_eq!(
        *bench.told.paths.lock().unwrap(),
        HashSet::from([bench.file(&Profile::Default)])
    );

    assert_eq!(bench.turn(), 1);
    let rows = bench.rows();
    let took = committed.elapsed();
    assert_eq!(
        rows,
        [(
            "default:s1".to_owned(),
            json!("Mac mini"),
            json!("Plan the week"),
            json!(0)
        )]
    );
    assert!(took <= SOON, "{took:?}");
    eprintln!("first mirror: listed {took:?} after its commit");
}

/// The contrast: with the writer's word withheld too, neither the look
/// between passes nor a pass finds a first mirror.
#[test]
fn a_first_mirror_nobody_told_core_of_is_not_found() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Plan the week")], 1_000, 1);
    assert!(!bench.core.look(), "the look sees only logs core knows");
    bench.silent_pass();
    assert!(bench.rows().is_empty());
    // Told, it is read.
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.ids(), ["default:s1"]);
}

#[test]
fn a_wal_only_update_is_read_without_file_events() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Plan the week")], 1_000, 1);
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1"]);
    let path = bench.file(&Profile::Default);
    let [main, wal] = stamps(&path);

    // A second session: the writer's connection stays open, so the
    // commit goes to the write-ahead log and the main file is untouched.
    std::thread::sleep(Duration::from_millis(20));
    bench.lists(
        &Profile::Default,
        &[("s1", "Plan the week"), ("s2", "Fix the build")],
        2_000,
        1,
    );
    let committed = *bench.told.at.lock().unwrap().last().unwrap();
    let [main_after, wal_after] = stamps(&path);
    assert_eq!(main_after, main, "the main file changed");
    assert_ne!(wal_after, wal, "the write-ahead log did not");

    assert_eq!(bench.turn(), 1);
    let ids = bench.ids();
    let took = committed.elapsed();
    assert_eq!(ids, ["default:s1", "default:s2"]);
    assert!(took <= SOON, "{took:?}");
    eprintln!("wal-only update: listed {took:?} after its commit");
}

#[test]
fn a_mirror_outside_the_64_newest_logs_is_read() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Plan the week")], 1_000, 1);
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1"]);

    // Seventy logs core knows, each with an event after the mirror was
    // last written: the 64 the look between passes checks are all newer
    // than the mirror.
    std::thread::sleep(Duration::from_millis(1_100));
    let folder = bench.data.join("claude");
    std::fs::create_dir_all(&folder).unwrap(); // plain directory
    for n in 0..70 {
        let id = format!("{n:08}-2222-3333-4444-555555555555");
        let timestamp = chrono::Utc::now().to_rfc3339();
        let line = json!({"type": "assistant", "sessionId": id, "timestamp": timestamp,
            "message": {"id": format!("m{n}"), "model": "claude-opus-4-5", "role": "assistant",
                "content": [{"type": "text", "text": "Done."}],
                "usage": {"input_tokens": 1, "output_tokens": 1,
                    "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}}});
        std::fs::write(folder.join(format!("{id}.jsonl")), format!("{line}\n")).unwrap();
    }
    bench.core.request("refresh", json!({})).unwrap();
    for _ in 0..20 {
        if !bench.core.indexing() {
            break;
        }
        bench.silent_pass();
    }
    let query = json!({"scope": "all", "search": ""});
    let known = bench.core.request("sessions", query).unwrap();
    assert!(known["items"].as_array().unwrap().len() >= 71);

    bench.lists(
        &Profile::Default,
        &[("s1", "Plan the week"), ("s2", "Fix the build")],
        2_000,
        1,
    );
    let committed = *bench.told.at.lock().unwrap().last().unwrap();
    // The look between passes does not reach it, and a pass that hears
    // of no change does not read it.
    assert!(!bench.core.look(), "the mirror is among the logs looked at");
    bench.silent_pass();
    assert_eq!(bench.ids(), ["default:s1"]);

    assert_eq!(bench.turn(), 1);
    let ids = bench.ids();
    let took = committed.elapsed();
    assert_eq!(ids, ["default:s1", "default:s2"]);
    assert!(took <= SOON, "{took:?}");
    eprintln!("outside the 64 newest: listed {took:?} after its commit");
}

#[test]
fn a_live_list_write_after_a_durable_row_changes_nothing_and_tells_nothing() {
    let bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s0", "Made by a poll")], 1_000, 1);
    // A durable row, set up by hand as the far database gives one.
    let db = rusqlite::Connection::open(bench.file(&Profile::Default)).unwrap();
    db.execute_batch(
        "INSERT INTO sessions (id, title, model, source, started_at, input_tokens, output_tokens)
            VALUES ('s1', 'Fix the build', 'claude-test', 'cli', 1760000000, 100, 20);
         INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens, output_tokens, first_seen, last_seen)
            VALUES ('s1', 'claude-test', 'anthropic', 100, 20, 1760000010, 1760000600);
         INSERT INTO hud_rows VALUES ('s1', 1, 1760000700000, 'print-1', 0);",
    )
    .unwrap();
    drop(db);
    let (before, tells) = (bench.dump(&Profile::Default), bench.tells());

    // The live list names both, each said otherwise.
    bench.lists(
        &Profile::Default,
        &[("s0", "Another"), ("s1", "Another")],
        2_000,
        1,
    );
    assert_eq!(bench.dump(&Profile::Default), before);
    assert_eq!(bench.tells(), tells, "nothing was committed");

    // And a writer that begins anew, knowing nothing of the file. The
    // one before it told core once more, of the file it closed.
    bench.writer.stop();
    let tells = tells + 1;
    assert_eq!(bench.tells(), tells);
    let writer = writer(&bench.data, "Mac mini", &bench.told, &bench.number);
    writer.listed(
        listing(
            &Profile::Default,
            &[("s0", "Again"), ("s1", "Again")],
            3_000,
        ),
        1,
    );
    writer.settle();
    assert_eq!(bench.dump(&Profile::Default), before);
    assert_eq!(bench.tells(), tells);
    // It looked into the file and wrote nothing; closing the file is
    // told, as every close is, and the rows are as they were.
    writer.stop();
    assert_eq!(bench.tells(), tells + 1);
    assert_eq!(bench.dump(&Profile::Default), before);
}

#[test]
fn of_two_writes_queued_out_of_time_order_the_older_is_dropped() {
    let mut bench = bench("Mac mini");
    // The newer one is queued first.
    bench.writer.listed(
        listing(&Profile::Default, &[("new", "Read later")], 2_000),
        1,
    );
    bench.writer.listed(
        listing(&Profile::Default, &[("old", "Read sooner")], 1_000),
        1,
    );
    // One read at the same moment as the newest is no newer.
    bench.writer.listed(
        listing(&Profile::Default, &[("same", "Read then")], 2_000),
        1,
    );
    bench.writer.settle();
    bench.turn();
    assert_eq!(bench.ids(), ["default:new"]);
    assert_eq!(bench.tells(), 1);

    // Each profile has its own order: an older read of another profile
    // is that profile's newest.
    bench.lists(&reports(), &[("r1", "Report")], 500, 1);
    bench.turn();
    assert_eq!(bench.ids(), ["default:new", "reports:r1"]);
    // And a newer one is written.
    bench.lists(&Profile::Default, &[("newer", "Read last")], 3_000, 1);
    bench.turn();
    assert_eq!(bench.ids(), ["default:new", "default:newer", "reports:r1"]);
}

#[test]
fn a_write_read_under_a_route_number_that_is_not_current_is_dropped() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Under the first")], 1_000, 1);
    let before = bench.dump(&Profile::Default);
    let tells = bench.tells();

    // The route changed: what was read under the old number arrives
    // late, and what is read under the new one is written.
    bench.number.now.store(2, Ordering::Release);
    bench.lists(&Profile::Default, &[("late", "Under the first")], 2_000, 1);
    assert_eq!(bench.dump(&Profile::Default), before);
    assert_eq!(bench.tells(), tells);
    // The dropped write did not count as the newest either.
    bench.lists(&Profile::Default, &[("s2", "Under the second")], 1_500, 2);
    assert_eq!(bench.tells(), tells + 1);
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1", "default:s2"]);
    assert!(bench
        .log()
        .contains("mirror dropped profile=default why=route"));
}

/// The number goes up after the writer looked at it and before it
/// commits: the transaction is given up, with nothing written.
#[test]
fn a_route_that_changes_before_the_commit_leaves_nothing_written() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Under the first")], 1_000, 1);
    let before = bench.dump(&Profile::Default);
    let (tells, asked) = (bench.tells(), bench.number.asked.load(Ordering::Acquire));

    // The next write asks twice: when it is taken, and to commit.
    bench.number.raise_at.store(asked + 2, Ordering::Release);
    bench.lists(&Profile::Default, &[("late", "Under the first")], 2_000, 1);
    assert_eq!(bench.number.asked.load(Ordering::Acquire), asked + 2);
    assert_eq!(bench.number.now.load(Ordering::Acquire), 2);
    assert_eq!(bench.dump(&Profile::Default), before);
    assert_eq!(bench.tells(), tells);
    // The session is not taken for held: under the new number it gets
    // its row.
    bench.lists(&Profile::Default, &[("late", "Under the second")], 3_000, 2);
    bench.turn();
    assert_eq!(bench.ids(), ["default:late", "default:s1"]);
}

#[test]
fn a_new_label_reaches_every_mirror_of_the_machine_and_every_row() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "One"), ("s2", "Two")], 1_000, 1);
    bench.lists(&reports(), &[("r1", "Report")], 1_000, 1);
    bench.turn();
    let shown =
        |bench: &mut Bench| -> Vec<Value> { bench.rows().into_iter().map(|row| row.1).collect() };
    assert_eq!(shown(&mut bench), vec![json!("Mac mini"); 3]);

    bench.writer.relabel("Studio");
    bench.writer.settle();
    assert_eq!(bench.turn(), 2, "both mirrors were told");
    assert_eq!(shown(&mut bench), vec![json!("Studio"); 3]);
    assert_eq!(bench.ids(), ["default:s1", "default:s2", "reports:r1"]);

    // The same label again writes nothing.
    let tells = bench.tells();
    bench.writer.relabel("Studio");
    bench.writer.settle();
    assert_eq!(bench.tells(), tells);

    // A writer that begins while the machine goes by another label than
    // its mirrors say (its entry was renamed while it was not followed)
    // writes it before anything else.
    bench.writer.stop();
    let writer = writer(&bench.data, "Lab", &bench.told, &bench.number);
    writer.settle();
    assert_eq!(bench.turn(), 2);
    assert_eq!(shown(&mut bench), vec![json!("Lab"); 3]);
    writer.stop();
}

#[test]
fn a_writer_that_was_stopped_writes_nothing_more() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "Before")], 1_000, 1);
    bench.turn();
    let before = bench.dump(&Profile::Default);
    let path = bench.file(&Profile::Default);

    // Stopped: its file is closed by the time `stop` returns, with the
    // write-ahead log folded in, and core was told of that.
    bench.writer.stop();
    let stamp = std::fs::metadata(&path).unwrap();
    let stamp = (stamp.len(), stamp.modified().unwrap());
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    assert!(!wal.exists(), "the write-ahead log is still there");
    assert_eq!(bench.turn(), 1);
    let tells = bench.tells();

    bench
        .writer
        .listed(listing(&Profile::Default, &[("s2", "After")], 2_000), 1);
    bench.writer.relabel("Studio");
    bench.writer.settle();
    bench.writer.stop();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(bench.dump(&Profile::Default), before);
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!((after.len(), after.modified().unwrap()), stamp);
    assert_eq!(bench.tells(), tells);
    // Its sessions stay listed.
    bench.silent_pass();
    assert_eq!(bench.ids(), ["default:s1"]);
}

#[test]
fn a_poll_that_lists_nothing_new_writes_nothing_and_tells_nothing() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "One")], 1_000, 1);
    let stamp = stamps(&bench.file(&Profile::Default));
    for read_ms in 2..40 {
        bench.lists(&Profile::Default, &[("s1", "One")], 1_000 + read_ms, 1);
        bench.lists(&reports(), &[], 1_000 + read_ms, 1);
    }
    assert_eq!(bench.tells(), 1);
    assert_eq!(stamps(&bench.file(&Profile::Default)), stamp);
    // A profile with no live session has no mirror made for it.
    assert!(!bench.file(&reports()).exists());
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1"]);
}

#[test]
fn a_mirror_that_is_gone_from_the_disk_is_made_anew() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "One")], 1_000, 1);
    let folder = bench.file(&Profile::Default);
    std::fs::remove_dir_all(folder.parent().unwrap()).unwrap();
    bench.lists(&Profile::Default, &[("s1", "One")], 2_000, 1);
    assert!(bench.file(&Profile::Default).is_file());
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1"]);
}

/// Writes one made-up machine's mirror into a data folder named by hand,
/// with the writer the app uses: for looking at what an app that is
/// running makes of a mirror written beside it (ticket 34's evidence).
/// `AGENT_HUD_MIRROR_DATA` names the folder, and `AGENT_HUD_MIRROR_SECOND`
/// asks for the second machine in place of the first. Nothing is told
/// to anyone: the app that reads the folder is another process.
#[test]
#[ignore = "writes where it is told: run by hand"]
fn writes_a_made_up_mirror_where_it_is_told() {
    let Some(data) = std::env::var_os("AGENT_HUD_MIRROR_DATA") else {
        return;
    };
    let data = PathBuf::from(data);
    let (install, label, session, title) = match std::env::var_os("AGENT_HUD_MIRROR_SECOND") {
        None => (
            "0123456789abcdef0123456789abcdef",
            "Mac mini Lab",
            "lab-a1",
            "Plan the week",
        ),
        Some(_) => (
            "fedcba9876543210fedcba9876543210",
            "Studio Lab",
            "lab-b1",
            "Water the plants",
        ),
    };
    let key = machine_key(Some(install), "/far/.hermes").unwrap();
    let number = Arc::new(Number::default());
    let untold: Changed = Arc::new(|_| {});
    let log = Log::new(&data).of(Some("mirror"));
    let writer = Writer::start(&data, (&key, label), number.clone(), untold, log).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    writer.listed(listing(&Profile::Default, &[(session, title)], now), 0);
    writer.settle();
    writer.stop();
    println!(
        "WROTE {} at {}",
        writer.file(&Profile::Default).display(),
        chrono::Utc::now().to_rfc3339()
    );
}
