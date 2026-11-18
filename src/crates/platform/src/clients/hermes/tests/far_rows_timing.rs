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
//! A pass costs more the more sessions core lists, whatever it reads:
//! with 5,000 it is about 0.3 s of this test build's CPU time, most of
//! it the snapshot. So on a machine whose load is several times its
//! cores, one pass alone is over the 2 s, and these cases say so. Each
//! line ends with the machine's load.
use super::{
    super::Pace,
    fake::Fake,
    machines::{
        fleet, here,
        mirrors::{far, works},
        ssh, until, Fleet, INSTALL,
    },
};
use agent_hud_agents::hermes::gateway::{Listed, Listing, Profile};
use std::{
    collections::{BTreeMap, HashSet},
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
    fleet.start_at(Pace::default());
    let key = fleet.connected("Mini");
    until("the polls run", || !gateway.world().listed.is_empty());
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
