//! How long a far session's row takes to show, from its turn starting on
//! the far machine to core listing it, held against the row bounds of
//! the phase 2 design (spec section 10): 6 s when the gateway had a busy
//! session, 15 s when it had none.
//!
//! Everything is as in the app: the machines the app runs, the app's own
//! pace (`Pace::default()`), the route over SSH through the lab's
//! `sshd`, the mirrors' writer, and a real core that reads the mirrors'
//! folder. Both of what could make a row sooner are withheld. The
//! gateway never says `sessions.changed`, so only the polls find the
//! turn. And core has no file watcher: it hears that no file changed
//! before every pass, so the row is read only because the writer said
//! which file it wrote. The measured time is the wait for a poll, the
//! poll, the mirror write and the pass.
//!
//! The same for a far session's count (ticket 35): from Hermes saying
//! a session has used more to its row showing that, held to the busy
//! row's bound, with the mirrors' writer spacing its commits of counts
//! as it does in the app. And the busy row's case once more while the
//! counts of four sessions move all the time, so the rows are measured
//! with the count writes going on beside them.
//!
//! Twenty runs of each, and a miss fails the test. Each prints its
//! numbers as one line that begins with `TIMING`. They wait on the
//! clock, the second for minutes, so they are run by hand:
//! `cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_rows_timing -- --ignored --nocapture --test-threads 2`.
//!
//! **A mirror that has grown (ticket 54).** The same two cases with
//! 1,000 and with 5,000 rows in the mirror already, half of them as the
//! live list left them and half as the far database gives them, with
//! tokens and a usage row, which is what ticket 37 adds. These also
//! hold each row against the 2 s there are from its commit to its
//! listing. The collector's turns are the app's
//! (`host/src/service/collector.rs`): one at once when a writer asks,
//! and one every two seconds while core says logs wait. The cases with
//! an empty mirror take a turn only when a writer asks, as before.
//!
//! **What changes in a grown mirror (ticket 60).** With the same 1,000
//! and 5,000 rows: a count that moves in a session that works, held to
//! the busy row's bound and to the 2 s from its commit; and the
//! machine's label, which changes every row, held to the same two from
//! the writer being told to every row showing it. Each shows after the
//! one pass its write asks for, and the cases fail where core says
//! logs still wait after it.
//!
//! A pass costs more the more sessions core lists, whatever it reads:
//! with 5,000 it is about 0.3 s of this test build's CPU time, most of
//! it the snapshot. So on a machine whose load is several times its
//! cores, one pass alone is over the 2 s, and these cases say so. Each
//! line ends with the machine's load.
use super::{
    super::Pace,
    fake::Fake,
    machines::{
        counts::uses,
        fleet, here,
        mirrors::{far, works},
        ssh, until, Fleet, INSTALL,
    },
};
use agent_hud_agents::hermes::gateway::{Listed, Listing, Profile};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// A row when the gateway had a busy session.
const BUSY: Duration = Duration::from_secs(6);
/// A row when it had none.
const IDLE: Duration = Duration::from_secs(15);
const RUNS: u32 = 20;
/// From a row's commit to its listing (ticket 34).
const SOON: Duration = Duration::from_secs(2);
/// How far apart the collector's passes are while core says logs wait.
const SPACING: Duration = Duration::from_secs(2);

fn following() -> Option<(Fleet, Fake, String)> {
    let mut fleet = fleet(1)?;
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    let mut pace = Pace::default();
    // Asked for by the same word as in `far_timing.rs` (ticket 37): the
    // far machine's history is read back to back while the rows are
    // timed, four hundred stored sessions of it, into the same mirror.
    let reads = std::env::var_os(super::far_timing::HISTORY_READS).is_some();
    if reads {
        let db = super::machines::history::far_db(&fleet.hosts[0].root);
        for n in 0..400 {
            let id = format!("s-stored-{n:03}");
            super::machines::history::session(&db, &id, 1_600_000_000.0 + f64::from(n));
            super::machines::history::used(&db, &id, "model-a", "", 100);
        }
        Box::leak(Box::new(db));
        pace.history = super::super::history::Plan {
            first: Duration::ZERO,
            live: Duration::ZERO,
            idle: Duration::ZERO,
            page: 100,
            ..Default::default()
        };
    }
    fleet.start_at(pace);
    let key = fleet.connected("Mini");
    until("the polls run", || !gateway.world().listed.is_empty());
    if reads {
        let read = |log: &str| log.contains(" history ") && log.contains(" ok exit=0 ");
        until("the history reads run", || read(&fleet.log()));
    }
    Some((fleet, gateway, key))
}

/// How many polls the gateway has seen begin.
fn polls(gateway: &Fake) -> usize {
    gateway.world().listed.len()
}

/// A turn starts in `session`, with no hint; how long until its row is
/// listed. The collector takes its turn as soon as a writer asks.
fn started(
    fleet: &mut Fleet,
    gateway: &Fake,
    key: &str,
    session: &str,
    limit: Duration,
) -> Duration {
    let id = far(key, session);
    let began = Instant::now();
    works(gateway, session, "Started on the far machine");
    loop {
        if fleet.turn() > 0 && fleet.far_rows().iter().any(|row| row.id == id) {
            return began.elapsed();
        }
        let took = began.elapsed();
        assert!(took <= limit, "no row for {session} after {took:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Prints the numbers of one case, and fails it where any is over
/// `bound`.
fn held(name: &str, took: Vec<Duration>, bound: Duration) {
    let (over, runs) = (said(name, took.clone(), bound), took.len());
    assert!(over == 0, "{name}: {over} of {runs} over {bound:?}");
}

/// Prints the numbers of one case; how many are over `bound`.
fn said(name: &str, mut took: Vec<Duration>, bound: Duration) -> usize {
    took.sort();
    let ms = |time: &Duration| time.as_millis();
    let over = took.iter().filter(|time| **time > bound).count();
    let (least, middle, longest) = (&took[0], &took[took.len() / 2], &took[took.len() - 1]);
    let load = std::process::Command::new("/usr/bin/uptime").output();
    let load = load.map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    println!(
        "TIMING {name} n={} least_ms={} middle_ms={} most_ms={} over={over} bound_ms={} all_ms={:?} | {}",
        took.len(),
        ms(least),
        ms(middle),
        ms(longest),
        ms(&bound),
        took.iter().map(ms).collect::<Vec<_>>(),
        load.unwrap_or_default(),
    );
    over
}

/// Puts `rows` rows into the mirror of the default profile of the
/// machine `key`, through its writer, and has core read them all. Every
/// other one is then made as the far database gives it, by hand:
/// durable, with tokens and one usage row.
fn grown(fleet: &mut Fleet, key: &str, rows: usize) {
    let said = |n: usize| Listed {
        title: Some(format!("Held already {n}")),
        model: Some("claude-test".into()),
        source: Some("telegram".into()),
        started_at: Some(1_759_000_000.0 + n as f64),
    };
    let sessions = (0..rows).map(|n| (format!("held-{n:05}"), said(n)));
    let listing = Listing {
        read_ms: crate::hooks::now_ms(),
        profiles: BTreeMap::from([(Profile::Default, sessions.collect())]),
    };
    let (writer, path) = (fleet.writer("a"), fleet.mirror(key, "default"));
    writer.listed(listing, fleet.number());
    writer.settle();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.busy_timeout(Duration::from_secs(10)).unwrap();
    db.execute_batch(
        "BEGIN IMMEDIATE;
         UPDATE sessions SET input_tokens=100, output_tokens=20, ended_at=started_at+600
            WHERE id LIKE 'held-%' AND rowid % 2 = 0;
         INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens,
            output_tokens, first_seen, last_seen)
            SELECT id, 'claude-test', 'anthropic', 100, 20, started_at+10, started_at+600
            FROM sessions WHERE id LIKE 'held-%' AND rowid % 2 = 0;
         UPDATE hud_rows SET durable=1, print='print-1' WHERE session_id IN
            (SELECT id FROM sessions WHERE id LIKE 'held-%' AND rowid % 2 = 0);
         COMMIT;",
    )
    .unwrap();
    drop(db);
    fleet.told.lock().unwrap().insert(path);
    fleet.turn();
    let mut passes = 0;
    while fleet.core.indexing() {
        fleet.snapshot();
        passes += 1;
        assert!(passes < 1_000, "core never finished reading the mirror");
    }
    assert!(fleet.count() >= rows, "{} of {rows} listed", fleet.count());
    println!("a mirror of {rows} rows, read by core in {passes} passes after the first");
}

/// The app's collector as far as a mirror's rows go: when it last made
/// a pass that nothing asked for.
struct Collector {
    passed: Instant,
}

impl Collector {
    fn new() -> Self {
        Self {
            passed: Instant::now() - SPACING,
        }
    }

    /// Takes the turn that is due, where one is: at once where a writer
    /// asked for one, which reads what it wrote; and a pass with nothing
    /// to prompt it where core says logs wait and the last such pass is
    /// two seconds ago. True where a pass was made.
    fn turn(&mut self, fleet: &mut Fleet) -> bool {
        if fleet.asked() {
            fleet.turn();
            return true;
        }
        if fleet.core.indexing() && self.passed.elapsed() >= SPACING {
            self.passed = Instant::now();
            fleet.snapshot();
            return true;
        }
        false
    }
}

/// A turn starts in `session`, with no hint, while the mirror holds
/// many rows. How long until its row is listed, and how long that was
/// after the row's commit.
fn started_in_a_grown_mirror(
    (fleet, collector): (&mut Fleet, &mut Collector),
    gateway: &Fake,
    key: &str,
    session: &str,
    limit: Duration,
) -> (Duration, Duration) {
    let (id, title) = (far(key, session), format!("Began as {session}"));
    // Whatever was told before is read first.
    while collector.turn(fleet) && fleet.asked() {}
    *fleet.told_at.lock().unwrap() = None;
    let began = Instant::now();
    works(gateway, session, &title);
    loop {
        if collector.turn(fleet) {
            // The row is listed, or not, when the pass ends: the clock is
            // read then, and the look at the list is not part of it.
            let committed = *fleet.told_at.lock().unwrap();
            let took = (began.elapsed(), committed.map(|at| at.elapsed()));
            if fleet.lists(&id, &title) {
                return (took.0, took.1.expect("a row nobody told of"));
            }
        }
        let took = began.elapsed();
        assert!(took <= limit, "no row for {session} after {took:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Prints and judges both numbers of the runs in a grown mirror.
fn held_grown(name: &str, rows: usize, took: Vec<(Duration, Duration)>, bound: Duration) {
    let (row, commit): (Vec<Duration>, Vec<Duration>) = took.into_iter().unzip();
    let distinct: HashSet<Duration> = row.iter().copied().collect();
    assert_eq!(distinct.len().min(2), 2, "the runs were one run");
    // Both are printed before either is judged.
    let late = said(&format!("{name}_with_{rows}_rows"), row, bound);
    let commit_name = format!("{name}_with_{rows}_rows_commit_to_listed");
    let slow = said(&commit_name, commit, SOON);
    assert!(late == 0, "{name} with {rows} rows: {late} over {bound:?}");
    assert!(slow == 0, "{commit_name}: {slow} over {SOON:?}");
}

fn busy_with(rows: usize) {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    works(&gateway, "s-busy", "Busy throughout");
    fleet.listed(&far(&key, "s-busy"));
    grown(&mut fleet, &key, rows);
    let mut collector = Collector::new();
    let mut took = Vec::new();
    for run in 0..RUNS {
        let seen = polls(&gateway);
        until("the next poll begins", || polls(&gateway) > seen);
        std::thread::sleep(Duration::from_secs(1) * run / RUNS);
        let session = format!("s-turn-{run}");
        let both = (&mut fleet, &mut collector);
        took.push(started_in_a_grown_mirror(
            both,
            &gateway,
            &key,
            &session,
            BUSY * 2,
        ));
    }
    held_grown("row_when_the_gateway_was_busy", rows, took, BUSY);
}

fn idle_with(rows: usize) {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    // The mirror is made by the first row there is.
    works(&gateway, "s-first", "The first there was");
    fleet.listed(&far(&key, "s-first"));
    grown(&mut fleet, &key, rows);
    let mut collector = Collector::new();
    let mut took = Vec::new();
    for run in 0..RUNS {
        gateway.world().streams.clear();
        let seen = polls(&gateway);
        // The collector goes on with its passes while the polls slow.
        until("two polls find nothing", || {
            collector.turn(&mut fleet);
            polls(&gateway) >= seen + 2
        });
        let wait = Duration::from_millis(100) + Duration::from_secs(10) * run / RUNS;
        let until_then = Instant::now() + wait;
        while Instant::now() < until_then {
            collector.turn(&mut fleet);
            std::thread::sleep(Duration::from_millis(2));
        }
        let session = format!("s-first-{run}");
        let both = (&mut fleet, &mut collector);
        took.push(started_in_a_grown_mirror(
            both,
            &gateway,
            &key,
            &session,
            IDLE * 2,
        ));
    }
    held_grown("row_when_it_was_idle", rows, took, IDLE);
}

/// What core lists of the far session `id`, found by its title: the
/// tokens its row shows.
fn tokens_of(fleet: &mut Fleet, id: &str, title: &str) -> serde_json::Value {
    let query = json!({"scope": "all", "search": title});
    let found = fleet.core.request("sessions", query).unwrap();
    let items = found["items"].as_array().unwrap();
    let row = items.iter().find(|item| item["id"] == id);
    row.map_or(serde_json::Value::Null, |row| row["tokens"].clone())
}

/// Hermes says `session` has used `total` by now, while the mirror
/// holds many rows. How long until its row shows that, and how long
/// that was after the count's commit.
fn counted_in_a_grown_mirror(
    (fleet, collector): (&mut Fleet, &mut Collector),
    gateway: &Fake,
    (key, session, title): (&str, &str, &str),
    total: u64,
    limit: Duration,
) -> (Duration, Duration) {
    let id = far(key, session);
    while collector.turn(fleet) && fleet.asked() {}
    *fleet.told_at.lock().unwrap() = None;
    let began = Instant::now();
    uses(gateway, session, total - total / 5, total / 5);
    loop {
        if collector.turn(fleet) {
            let committed = *fleet.told_at.lock().unwrap();
            let took = (began.elapsed(), committed.map(|at| at.elapsed()));
            if tokens_of(fleet, &id, title) == json!(total) {
                return (took.0, took.1.expect("a count nobody told of"));
            }
        }
        let took = began.elapsed();
        assert!(
            took <= limit,
            "{session} does not show {total} after {took:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A count that moved, in a session that works, in a mirror that holds
/// `rows` rows (ticket 60): from Hermes saying it to the row showing
/// it, the writer's spacing of count commits included, and from the
/// count's commit to its listing.
fn count_with(rows: usize) {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    let title = "Busy and counted throughout";
    works(&gateway, "s-busy", title);
    fleet.listed(&far(&key, "s-busy"));
    grown(&mut fleet, &key, rows);
    let mut collector = Collector::new();
    let mut took = Vec::new();
    for run in 0..RUNS {
        let seen = polls(&gateway);
        until("the next poll begins", || {
            collector.turn(&mut fleet);
            polls(&gateway) > seen
        });
        std::thread::sleep(Duration::from_secs(1) * run / RUNS);
        let total = 1_000 * (u64::from(run) + 1);
        let both = (&mut fleet, &mut collector);
        let whose = (key.as_str(), "s-busy", title);
        took.push(counted_in_a_grown_mirror(
            both,
            &gateway,
            whose,
            total,
            BUSY * 2,
        ));
        // A pass after the one that showed it would be the old defect.
        assert!(!fleet.core.indexing(), "logs wait after a count");
    }
    held_grown("count_when_the_gateway_was_busy", rows, took, BUSY);
}

/// The machine is given another label while its mirror holds `rows`
/// rows (ticket 60): from the writer being told to every one of its
/// rows showing the label, and from the label's commit to that.
fn label_with(rows: usize) {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    works(&gateway, "s-busy", "Busy throughout");
    fleet.listed(&far(&key, "s-busy"));
    grown(&mut fleet, &key, rows);
    let labelled = |fleet: &mut Fleet, label: &str| {
        let query = json!({"scope": "all", "search": label});
        let found = fleet.core.request("sessions", query).unwrap();
        found["total"].as_u64().unwrap() as usize
    };
    let all = labelled(&mut fleet, "Mini");
    assert!(all > rows, "{all} rows of {rows} say Mini");
    let writer = fleet.writer("a");
    let mut collector = Collector::new();
    let mut took = Vec::new();
    for run in 0..RUNS {
        // The polls and the collector go on as they do, a moment
        // longer each time.
        let wait =
            Instant::now() + Duration::from_millis(300) + Duration::from_secs(1) * run / RUNS;
        while Instant::now() < wait || (collector.turn(&mut fleet) && fleet.asked()) {
            collector.turn(&mut fleet);
            std::thread::sleep(Duration::from_millis(2));
        }
        *fleet.told_at.lock().unwrap() = None;
        let label = format!("Relabelled-{run:02}-of-twenty");
        let began = Instant::now();
        writer.relabel(&label);
        loop {
            if collector.turn(&mut fleet) {
                let committed = *fleet.told_at.lock().unwrap();
                let at = (began.elapsed(), committed.map(|at| at.elapsed()));
                if labelled(&mut fleet, &label) == all {
                    took.push((at.0, at.1.expect("a label nobody told of")));
                    break;
                }
            }
            let waited = began.elapsed();
            assert!(waited <= BUSY * 4, "no label after {waited:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!fleet.core.indexing(), "logs wait after a label");
    }
    held_grown("label_on_every_row", rows, took, BUSY);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn count_when_the_gateway_was_busy_with_1000_rows() {
    count_with(1_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn count_when_the_gateway_was_busy_with_5000_rows() {
    count_with(5_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn label_on_every_row_with_1000_rows() {
    label_with(1_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn label_on_every_row_with_5000_rows() {
    label_with(5_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_the_gateway_was_busy_with_1000_rows() {
    busy_with(1_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_the_gateway_was_busy_with_5000_rows() {
    busy_with(5_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_it_was_idle_with_1000_rows() {
    idle_with(1_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_it_was_idle_with_5000_rows() {
    idle_with(5_000);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_the_gateway_was_busy() {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    // One session works throughout: the polls keep their close watch.
    works(&gateway, "s-busy", "Busy throughout");
    fleet.listed(&far(&key, "s-busy"));
    let mut took = Vec::new();
    for run in 0..RUNS {
        // From the start of a poll, and a twentieth of the second
        // between two polls later each time: the worst moment is one.
        let seen = polls(&gateway);
        until("the next poll begins", || polls(&gateway) > seen);
        std::thread::sleep(Duration::from_secs(1) * run / RUNS);
        let session = format!("s-turn-{run}");
        took.push(started(&mut fleet, &gateway, &key, &session, BUSY * 2));
    }
    held("row_when_the_gateway_was_busy", took, BUSY);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_it_was_idle() {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    let mut took = Vec::new();
    for run in 0..RUNS {
        // Two polls in a row with nothing listed: the slow pace is on.
        // Then a twentieth of its ten seconds later each time.
        gateway.world().streams.clear();
        let seen = polls(&gateway);
        until("two polls find nothing", || polls(&gateway) >= seen + 2);
        std::thread::sleep(Duration::from_millis(100) + Duration::from_secs(10) * run / RUNS);
        let session = format!("s-first-{run}");
        took.push(started(&mut fleet, &gateway, &key, &session, IDLE * 2));
    }
    held("row_when_it_was_idle", took, IDLE);
}

/// Hermes says `session` has used `total` by now; how long until its
/// row shows that. The collector takes its turn as soon as a writer
/// asks.
fn counted(
    fleet: &mut Fleet,
    gateway: &Fake,
    (key, session): (&str, &str),
    total: u64,
    limit: Duration,
) -> Duration {
    let id = far(key, session);
    let began = Instant::now();
    uses(gateway, session, total - total / 5, total / 5);
    loop {
        if fleet.turn() > 0 {
            let row = fleet.far_rows().into_iter().find(|row| row.id == id);
            if row.is_some_and(|row| row.tokens == json!(total)) {
                return began.elapsed();
            }
        }
        let took = began.elapsed();
        assert!(
            took <= limit,
            "{session} does not show {total} after {took:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A count that moved, in a session that works.
#[test]
#[ignore = "waits on the clock: run by hand"]
fn count_when_the_gateway_was_busy() {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    works(&gateway, "s-busy", "Busy throughout");
    fleet.listed(&far(&key, "s-busy"));
    let mut took = Vec::new();
    for run in 0..RUNS {
        // From the start of a poll, a twentieth of the second between
        // two polls later each time. The runs come a few seconds apart,
        // so the writer's spacing has mostly passed for each: what a
        // count waits when it moves again at once is in the lab test of
        // one turn (`machines/counts.rs`).
        let seen = polls(&gateway);
        until("the next poll begins", || polls(&gateway) > seen);
        std::thread::sleep(Duration::from_secs(1) * run / RUNS);
        let total = 1_000 * (u64::from(run) + 1);
        took.push(counted(
            &mut fleet,
            &gateway,
            (&key, "s-busy"),
            total,
            BUSY * 2,
        ));
    }
    held("count_when_the_gateway_was_busy", took, BUSY);
    // The gateway was asked for the count at every poll, and no more.
    let (stats, lists) = {
        let world = gateway.world();
        let lists = world.calls("session.active_list");
        let own =
            |(_, frame): &&(usize, serde_json::Value)| frame["params"]["profile"] == "default";
        (
            world.calls("session.stats").len(),
            lists.iter().filter(own).count(),
        )
    };
    println!("COUNT CALLS session.stats={stats} polls={lists}");
    assert!(stats <= lists, "{stats} stats calls in {lists} polls");
    assert!(stats + 4 >= lists, "{stats} stats calls in {lists} polls");
}

/// A session's row, when the gateway had busy sessions whose counts
/// move at every poll: the row's bound with the count writes on.
#[test]
#[ignore = "waits on the clock: run by hand"]
fn row_when_the_gateway_was_busy_and_counts_move() {
    let Some((mut fleet, gateway, key)) = following() else {
        return;
    };
    let busy = ["s-busy-0", "s-busy-1", "s-busy-2", "s-busy-3"];
    for session in busy {
        works(&gateway, session, "Busy throughout");
        fleet.listed(&far(&key, session));
    }
    let over = AtomicBool::new(false);
    let took = std::thread::scope(|scope| {
        // Every session's count grows every 150 ms, for as long as the
        // rows are measured.
        scope.spawn(|| {
            let mut total = 100;
            while !over.load(Ordering::Acquire) {
                total += 7;
                for session in busy {
                    uses(&gateway, session, total, total / 10);
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        });
        let mut took = Vec::new();
        for run in 0..RUNS {
            let seen = polls(&gateway);
            until("the next poll begins", || polls(&gateway) > seen);
            std::thread::sleep(Duration::from_secs(1) * run / RUNS);
            let session = format!("s-turn-{run}");
            took.push(started(&mut fleet, &gateway, &key, &session, BUSY * 2));
        }
        over.store(true, Ordering::Release);
        took
    });
    held("row_when_the_gateway_was_busy_and_counts_move", took, BUSY);
    // The counts were written beside the rows, and reached them.
    let rows = fleet.far_rows();
    let counted = |row: &&super::machines::mirrors::Row| row.tokens.as_u64() > Some(100);
    assert_eq!(rows.iter().filter(counted).count(), busy.len(), "{rows:?}");
    let log = fleet.log();
    let commits = log.matches("counts=").count();
    let stats = gateway.world().calls("session.stats").len();
    println!("COUNT WRITES lines={commits} session.stats={stats}");
}
