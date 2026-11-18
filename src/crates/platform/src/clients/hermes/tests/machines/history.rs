//! A far machine's stored history, kept in step (ticket 37), in the lab
//! of `machines.rs`: the far Hermes root holds a `state.db` made from
//! Hermes's own schema with made-up rows, the history reader runs behind
//! the throwaway `sshd`, the machine's one writer writes the mirror, and
//! a real core reads it. The refresh ticks when a test says so.
use super::{super::fake::Fake, fleet, here, mirrors::far, ssh, until, Fleet, INSTALL};
use crate::clients::hermes::far_history::tests::schema;
use crate::clients::hermes::{far_history::Settings, history::History, history::Plan, Pace};
use agent_hud_agents::hermes::{far_history::refresh, gateway::Profile};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Long ago: outside the days the recent sweep looks at.
pub(super) const OLD: f64 = 1_600_000_000.0;

/// How the tests' refresh reads: by hand, and with a reader that lives
/// two seconds where a read never ends.
pub(super) fn plan() -> Plan {
    Plan {
        clock: false,
        reader: Settings {
            deadline: Duration::from_secs(2),
            margin: Duration::from_secs(2),
            local_wait: Duration::from_secs(15),
            ..Settings::default()
        },
        ..Plan::default()
    }
}

/// One far machine, `Mini`, followed, with a Hermes database in its
/// root that the far side's Hermes has open.
pub(super) struct Site {
    pub fleet: Fleet,
    pub gateway: Fake,
    pub key: String,
    /// The far side's own connection: the writer Hermes is.
    pub db: Connection,
    pub history: Arc<History>,
}

/// A Hermes database at `home`, in write-ahead mode and held open, as
/// one whose gateway runs.
pub(in crate::clients::hermes) fn far_db(home: &Path) -> Connection {
    let db = Connection::open(home.join("state.db")).expect("the far database");
    let mode: String = db
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    db.execute_batch(schema::HERMES).expect("Hermes's schema");
    db
}

/// A far session that began at `at` and ended a quarter of an hour on.
pub(in crate::clients::hermes) fn session(db: &Connection, id: &str, at: f64) {
    db.execute(
        "INSERT INTO sessions (id, source, model, started_at, ended_at, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, cwd, billing_provider, billing_base_url, billing_mode,
            title, last_activity_at)
         VALUES (?1, 'telegram', 'claude-fable-5', ?2, ?3, 0, 0, 0, 0, '/Users/far/work', 'anthropic',
            'https://api.anthropic.com', 'api', ?4, ?3)",
        (id, at, at + 900.0, format!("Made-up {id}")),
    )
    .expect("a session");
}

/// One usage row of `id`: `input` and 7, 11, 13 of the other counts, by
/// `model` through OpenRouter, for `task`.
pub(in crate::clients::hermes) fn used(
    db: &Connection,
    id: &str,
    model: &str,
    task: &str,
    input: i64,
) {
    db.execute(
        "INSERT INTO session_model_usage (session_id, model, billing_provider, billing_base_url, billing_mode,
            task, api_call_count, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
            first_seen, last_seen)
         VALUES (?1, ?2, 'openrouter', 'https://openrouter.ai/api/v1', 'api', ?3, 1, ?4, 7, 11, 13,
            (SELECT started_at + 10 FROM sessions WHERE id=?1), (SELECT started_at + 500 FROM sessions WHERE id=?1))",
        (id, model, task, input),
    )
    .expect("a usage row");
}

/// What the far database itself says `id` used, all four counts.
pub(super) fn direct(db: &Connection, id: &str) -> i64 {
    let sql = "SELECT TOTAL(input_tokens+output_tokens+cache_read_tokens+cache_write_tokens)
        FROM session_model_usage WHERE session_id=?1";
    db.query_row(sql, [id], |row| row.get::<_, f64>(0)).unwrap() as i64
}

pub(super) fn now() -> f64 {
    crate::hooks::now_ms() as f64 / 1000.0
}

impl Site {
    /// The site, its far database seeded by `seed` before anything reads
    /// it; None where this machine cannot run an `sshd`.
    pub(super) fn with(plan: Plan, seed: impl FnOnce(&Connection)) -> Option<Self> {
        let mut fleet = fleet(1)?;
        fleet.reads_mirrors();
        let db = far_db(&fleet.hosts[0].root);
        seed(&db);
        let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
        fleet.write(&[here(), ssh("a", "Mini", "m0")]);
        fleet.start_at(Pace {
            history: plan,
            ..super::pace()
        });
        let key = fleet.connected("Mini");
        let history = fleet.book().history_of("a").expect("its refresh");
        Some(Self {
            fleet,
            gateway,
            key,
            db,
            history,
        })
    }

    /// Ticks until no whole sweep is overdue: the first sync is done.
    pub(super) fn sync(&self) -> usize {
        let mut ticks = 0;
        until("the first whole sweep finishes", || {
            ticks += 1;
            let ticked = self.history.tick();
            ticked.reads > 0 && !ticked.overdue
        });
        ticks
    }

    /// Core reads whatever the writers told of, pass after pass, until
    /// the far rows it lists no longer change.
    pub(super) fn read(&mut self) -> Vec<super::mirrors::Row> {
        self.fleet.writer("a").settle();
        for _ in 0..500 {
            self.fleet.turn();
            if !self.fleet.asked() && !self.fleet.core.indexing() {
                break;
            }
        }
        self.fleet.far_rows()
    }

    pub(super) fn mirror(&self) -> PathBuf {
        self.fleet.mirror(&self.key, "default")
    }

    /// The ledger's total, as a snapshot says it.
    pub(super) fn total(&mut self) -> i64 {
        self.fleet.snapshot()["totals"]["total"].as_i64().unwrap()
    }

    /// The tokens core lists for the far session `id`.
    pub(super) fn tokens(&mut self, id: &str) -> Option<i64> {
        let id = far(&self.key, id);
        let row = self.fleet.far_rows().into_iter().find(|row| row.id == id);
        row.and_then(|row| row.tokens.as_i64())
    }

    /// The lines the history reader logged, each from its kind on.
    pub(super) fn reads(&self) -> Vec<String> {
        let log = self.fleet.log();
        let read = |line: &str| {
            line.split_once(" history ")
                .map(|(_, read)| read.to_owned())
        };
        log.lines().filter_map(read).collect()
    }
}

/// One query of a mirror, read without changing the file.
pub(super) fn mirrored<T: rusqlite::types::FromSql>(path: &Path, sql: &str) -> Vec<T> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = Connection::open_with_flags(path, flags).unwrap();
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement.query_map([], |row| row.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// Every row of a mirror's session tables, as text.
pub(super) fn rows(path: &Path) -> Vec<String> {
    let mut all = Vec::new();
    for table in ["sessions", "session_model_usage", "hud_rows"] {
        let columns: Vec<String> = mirrored(
            path,
            &format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid"),
        );
        let quoted: Vec<String> = columns
            .iter()
            .map(|name| format!("quote({name})"))
            .collect();
        let sql = format!("SELECT {} FROM {table} ORDER BY 1", quoted.join("||','||"));
        all.extend(
            mirrored::<String>(path, &sql)
                .into_iter()
                .map(|row| format!("{table} {row}")),
        );
    }
    all
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_ended_far_session_appears_with_its_per_model_usage() {
    let seed = |db: &Connection| {
        session(db, "s-ended", now() - 3_600.0);
        used(db, "s-ended", "model-a", "", 100);
        used(db, "s-ended", "model-b", "", 60);
        session(db, "s-old", OLD);
        used(db, "s-old", "model-a", "", 40);
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    let before = site.total();
    assert!(site.read().is_empty());
    site.sync();
    let listed = site.read();
    println!("far rows: {listed:?}\n{}", site.reads().join("\n"));
    // Each session's tokens are what the lab database says it used.
    let (ended, old) = (direct(&site.db, "s-ended"), direct(&site.db, "s-old"));
    assert_eq!((ended, old), (100 + 60 + 2 * 31, 40 + 31));
    assert_eq!(site.tokens("s-ended"), Some(ended));
    assert_eq!(site.tokens("s-old"), Some(old));
    assert!(listed.iter().all(|row| row.machine == "Mini"), "{listed:?}");
    // The ledger holds them once.
    assert_eq!(site.total() - before, ended + old);
    // Per model, with the whole identity of each usage row.
    let usage: Vec<String> = mirrored(
        &site.mirror(),
        "SELECT session_id||' '||model||' '||billing_provider||' '||billing_base_url||' '||billing_mode||' ['||task||'] '||input_tokens
         FROM session_model_usage ORDER BY 1",
    );
    assert_eq!(
        usage,
        [
            "s-ended model-a openrouter https://openrouter.ai/api/v1 api [] 100",
            "s-ended model-b openrouter https://openrouter.ai/api/v1 api [] 60",
            "s-old model-a openrouter https://openrouter.ai/api/v1 api [] 40",
        ]
    );
    let durable: Vec<i64> = mirrored(&site.mirror(), "SELECT durable FROM hud_rows");
    assert_eq!(durable, [1, 1]);

    // More ticks, and a core that starts anew on the same ledger: the
    // same rows, and nothing counted again.
    let held = rows(&site.mirror());
    for _ in 0..3 {
        site.history.tick();
    }
    assert_eq!(rows(&site.mirror()), held);
    assert!(!site.fleet.asked(), "a tick that changed no row told core");
    site.fleet.reads_mirrors();
    site.read();
    assert_eq!(site.total() - before, ended + old);
    assert_eq!(site.tokens("s-ended"), Some(ended));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_session_first_listed_live_then_read_from_the_database_becomes_durable_and_counts_once() {
    let seed = |db: &Connection| {
        session(db, "s-live", now() - 600.0);
        used(db, "s-live", "model-a", "", 500);
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    let before = site.total();
    // Listed live first: a row with nothing counted, and not durable.
    super::mirrors::works(&site.gateway, "s-live", "Plan the week");
    let id = far(&site.key, "s-live");
    assert_eq!(site.fleet.listed(&id).tokens, 0);
    assert_eq!(site.total(), before);
    let mark = |site: &Site| -> Vec<String> {
        mirrored(
            &site.mirror(),
            "SELECT s.rowid||' '||r.durable FROM sessions s JOIN hud_rows r ON r.session_id=s.id",
        )
    };
    assert_eq!(mark(&site), ["1 0"]);

    // Then read from the database: the same row, durable, counted once.
    site.sync();
    site.read();
    let used = direct(&site.db, "s-live");
    assert_eq!(mark(&site), ["1 1"]);
    assert_eq!(site.tokens("s-live"), Some(used));
    assert_eq!(site.total() - before, used);
    let title: Vec<String> = mirrored(&site.mirror(), "SELECT title FROM sessions");
    assert_eq!(title, ["Made-up s-live"]);

    // It goes on, and is read again while it is live: only what it grew
    // by is added.
    site.db
        .execute(
            "UPDATE session_model_usage SET input_tokens=input_tokens+250",
            [],
        )
        .unwrap();
    site.history.tick();
    site.read();
    assert_eq!(site.tokens("s-live"), Some(used + 250));
    assert_eq!(site.total() - before, used + 250);
    assert_eq!(mark(&site), ["1 1"]);
    // The polls list it again and again meanwhile, and change nothing.
    let held = rows(&site.mirror());
    site.fleet.writer("a").settle();
    site.history.tick();
    assert_eq!(rows(&site.mirror()), held);
    assert_eq!(site.total() - before, used + 250);
}

/// How many ticks it takes until `met`, a tick at a time, `most` at most.
pub(super) fn ticks_until(site: &Site, most: u64, mut met: impl FnMut() -> bool) -> u64 {
    for tick in 1..=most {
        site.history.tick();
        if met() {
            return tick;
        }
    }
    panic!("not within {most} ticks:\n{}", site.reads().join("\n"));
}

/// Thirty old sessions, each with one usage row.
fn thirty(db: &Connection) {
    for n in 0..30 {
        let id = format!("s-old-{n:02}");
        session(db, &id, OLD + f64::from(n));
        used(db, &id, "model-a", "", 100 + i64::from(n));
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_old_session_whose_usage_row_grows_is_picked_up_within_one_whole_sweep() {
    // A page of seven rows: a whole sweep of thirty is five pages.
    let plan = Plan { page: 7, ..plan() };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    site.sync();
    site.read();
    let before = (site.total(), site.tokens("s-old-11").unwrap());
    let stamps = |db: &Connection| -> String {
        let sql = "SELECT quote(started_at)||quote(ended_at)||quote(last_activity_at)||
            (SELECT quote(first_seen)||quote(last_seen) FROM session_model_usage WHERE session_id=id)
            FROM sessions WHERE id='s-old-11'";
        db.query_row(sql, [], |row| row.get(0)).unwrap()
    };
    let stamped = stamps(&site.db);
    // Its usage row grows, and nothing else of it changes: no stamp of
    // the session or of the row, so no read by time would find it.
    site.db
        .execute(
            "UPDATE session_model_usage SET input_tokens=input_tokens+1000 WHERE session_id='s-old-11'",
            [],
        )
        .unwrap();
    assert_eq!(stamps(&site.db), stamped);
    let grown = direct(&site.db, "s-old-11");
    let stored = |site: &Site| -> i64 {
        mirrored(
            &site.mirror(),
            "SELECT input_tokens FROM session_model_usage WHERE session_id='s-old-11'",
        )[0]
    };
    // One whole sweep, at one page a tick at least, and the read of the
    // session it found.
    let within = refresh::sweep_pages(30, plan.page) + 1;
    let from = site.reads().len();
    let took = ticks_until(&site, within, || stored(&site) == 111 + 1000);
    println!("picked up after {took} ticks, within {within}");
    site.read();
    assert_eq!(site.tokens("s-old-11"), Some(grown));
    assert_eq!(site.total() - before.0, 1000);
    assert_eq!(grown - before.1, 1000);
    // Found by the whole sweep: the recent one never returned it.
    let since: Vec<String> = (site.reads()[from..].iter())
        .filter(|read| read.starts_with("prints-since ok"))
        .cloned()
        .collect();
    assert!(!since.is_empty());
    assert!(
        since.iter().all(|read| read.contains(" prints=0 ")),
        "{since:?}"
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_new_usage_row_for_an_auxiliary_task_is_picked_up_with_its_task_in_the_key() {
    let plan = Plan { page: 7, ..plan() };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    site.sync();
    site.read();
    let before = (site.total(), site.tokens("s-old-20").unwrap());
    // An auxiliary task of a session that ended long ago: a new usage
    // row beside the one it has, with stamps as old as the session.
    used(&site.db, "s-old-20", "model-aux", "aux:title", 9);
    let tasks = |site: &Site| -> Vec<String> {
        mirrored(
            &site.mirror(),
            "SELECT task FROM session_model_usage WHERE session_id='s-old-20' ORDER BY 1",
        )
    };
    let within = refresh::sweep_pages(30, plan.page) + 1;
    ticks_until(&site, within, || tasks(&site) == ["", "aux:title"]);
    site.read();
    assert_eq!(site.tokens("s-old-20"), Some(before.1 + 9 + 31));
    assert_eq!(site.total() - before.0, 9 + 31);
    // Its task is in the record's key, as for a session of this Mac.
    let db = Connection::open(site.mirror()).unwrap();
    let (mut cursor, mut keys) = (Value::Null, Vec::new());
    for _ in 0..20 {
        let page =
            agent_hud_agents::hermes::transcript::read(&db, &site.mirror(), &cursor).unwrap();
        keys.extend(page.records.into_iter().map(|record| record.key));
        match page.cursor {
            Some(next) => cursor = next,
            None => break,
        }
    }
    let key = format!(
        "usage:{}:[\"model-aux\",\"openrouter\",\"https://openrouter.ai/api/v1\",\"api\",\"aux:title\"]",
        far(&site.key, "s-old-20")
    );
    assert!(keys.contains(&key), "{keys:?}");
}

/// Twelve sessions the gateway lists live, in the far database too.
fn live(site: &Site, count: usize) {
    for n in 0..count {
        let id = format!("s-live-{n:02}");
        session(&site.db, &id, now() - 60.0);
        used(&site.db, &id, "model-a", "", 10);
        super::mirrors::works(&site.gateway, &id, "Live");
    }
    until("the polls list them", || {
        site.fleet.writer("a").first(&Profile::Default, 0).len() >= count
    });
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_sweep_advances_every_tick_while_the_first_queue_is_kept_full() {
    // Four reads a tick, two sessions a read, four rows a page: the
    // first queue serves four sessions a tick, of twelve that are live,
    // so it is never empty; the sweep has one read in four.
    let plan = Plan {
        reads: 4,
        batch: 2,
        page: 4,
        ..plan()
    };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    live(&site, 12);
    site.sync();
    site.read();
    let before = site.total();
    site.db
        .execute(
            "UPDATE session_model_usage SET input_tokens=input_tokens+1000 WHERE session_id='s-old-05'",
            [],
        )
        .unwrap();
    let stored = |site: &Site| -> i64 {
        mirrored(
            &site.mirror(),
            "SELECT input_tokens FROM session_model_usage WHERE session_id='s-old-05'",
        )[0]
    };
    // One whole sweep of forty-two rows, and the read of what it found,
    // at the one read in four the sweep has in a tick of four.
    assert_eq!(refresh::sweep_reads(plan.reads), 1);
    let within = refresh::sweep_pages(42, plan.page) + 1;
    let (mut ticks, mut places) = (0, vec![site.history.sweep(&Profile::Default)]);
    while stored(&site) != 105 + 1000 {
        ticks += 1;
        assert!(ticks <= within, "not within {within} ticks: {places:?}");
        let from = site.reads().len();
        let ticked = site.history.tick();
        let read = site.reads()[from..].to_vec();
        // The tick was full, its first three reads the first queue's
        // and its fourth the sweep's.
        assert_eq!(ticked.reads, 4, "{read:?}");
        let kinds: Vec<&str> = read
            .iter()
            .filter_map(|read| read.split(' ').next())
            .collect();
        assert_eq!(kinds[..3], ["held", "sessions", "sessions"], "{read:?}");
        assert!(matches!(kinds[3], "prints" | "sessions"), "{read:?}");
        // And the sweep is somewhere else than a tick ago.
        let place = site.history.sweep(&Profile::Default);
        assert_ne!(
            Some(&place),
            places.last(),
            "the sweep stood still: {places:?}"
        );
        places.push(place);
    }
    println!("picked up after {ticks} ticks, within {within}; the sweep went {places:?}");
    site.read();
    assert_eq!(site.total() - before, 1000);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn with_the_last_sweep_older_than_the_set_time_the_sweep_goes_first_until_it_finishes() {
    // Forty-two rows are four pages of twelve. The set time is the
    // app's own, twenty-four hours.
    let plan = Plan {
        page: 12,
        batch: 2,
        ..plan()
    };
    let Some(site) = Site::with(plan, thirty) else {
        return;
    };
    live(&site, 12);
    site.sync();
    let done =
        |site: &Site| -> i64 { mirrored(&site.mirror(), "SELECT sweep_done_ms FROM hud_reads")[0] };
    let kinds = |site: &Site, from: usize| -> Vec<String> {
        let kind = |read: &String| read.split(' ').next().unwrap_or("").to_owned();
        site.reads()[from..].iter().map(kind).collect()
    };
    // In time: a tick begins with the first queue, which is full, and
    // its fourth read is the sweep's.
    let from = site.reads().len();
    site.history.tick();
    let in_time = kinds(&site, from);
    assert_eq!(
        in_time[..4],
        ["held", "sessions", "sessions", "prints"],
        "{in_time:?}"
    );
    let last = done(&site);

    // The last sweep that finished is a day and an hour old: every read
    // is the sweep's until it has finished, however full the first queue
    // is, and only then does the first queue get one.
    site.history.backdate(
        &Profile::Default,
        plan.sweep_within + Duration::from_secs(3600),
    );
    let at = site.history.sweep(&Profile::Default).0;
    let left = refresh::sweep_pages(42 - at as u64, plan.page) as usize;
    let from = site.reads().len();
    let ticked = site.history.tick();
    let overdue = kinds(&site, from);
    println!("in time {in_time:?}; overdue from rowid {at}, {left} pages left: {overdue:?}");
    assert!(
        overdue[..left].iter().all(|kind| kind == "prints"),
        "{overdue:?}"
    );
    let first = overdue
        .iter()
        .position(|kind| kind == "held")
        .expect("a held read");
    // The read after the sweep finished is the first queue's, but where
    // that read is the fourth of the tick, which is the sweep's by share.
    assert!(
        first == left || (first == left + 1 && (left + 1).is_multiple_of(4)),
        "{overdue:?}"
    );
    assert!(
        overdue[..first].iter().all(|kind| kind == "prints"),
        "{overdue:?}"
    );
    assert!(done(&site) > last);
    assert!(!ticked.overdue);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_status_shows_last_contact_and_the_last_whole_check_as_two_times() {
    // Two pages a sweep, and one sweep read a tick once it is in time.
    let plan = Plan {
        reads: 4,
        page: 20,
        ..plan()
    };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    let times = |site: &mut Site| -> (Option<String>, Option<String>) {
        let row = site.fleet.stands("Mini", "connected");
        let at = |key: &str| row[key].as_str().map(str::to_owned);
        (at("lastContactAt"), at("historyCheckedAt"))
    };
    // Followed, and not read yet: neither time.
    assert_eq!(times(&mut site), (None, None));
    site.sync();
    let mut first = (None, None);
    until("the status says both", || {
        first = times(&mut site);
        first.0.is_some() && first.1.is_some()
    });
    println!("after the first sync: {first:?}");
    // The whole check is never later than the last contact.
    assert!(first.0 >= first.1, "{first:?}");
    let stored: Vec<String> = mirrored(
        &site.mirror(),
        "SELECT quote(contact_ms)||' '||quote(sweep_done_ms) FROM hud_reads",
    );
    assert!(!stored[0].contains("NULL"), "{stored:?}");

    // A tick that reads and finishes no sweep: the contact moves, the
    // whole check stays where it was.
    std::thread::sleep(Duration::from_millis(1_100));
    let ticked = site.history.tick();
    assert!(ticked.reads > 0);
    assert_eq!(site.history.sweep(&Profile::Default).0, 20, "mid-sweep");
    let mut second = (None, None);
    until("the contact moves", || {
        second = times(&mut site);
        second.0 > first.0
    });
    assert_eq!(second.1, first.1);
    assert_ne!(second.0, second.1);

    // The sweep finishes: now the whole check moves too.
    std::thread::sleep(Duration::from_millis(1_100));
    ticks_until(&site, 3, || site.history.sweep(&Profile::Default).0 == 0);
    let mut third = (None, None);
    until("the whole check moves", || {
        third = times(&mut site);
        third.1 > second.1
    });
    println!("then {second:?}, then {third:?}");
    assert!(third.0 >= third.1);
}
