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
use super::{
    super::Pace,
    fake::Fake,
    machines::{
        fleet, here,
        mirrors::{far, works},
        ssh, until, Fleet, INSTALL,
    },
};
use std::time::{Duration, Instant};

/// A row when the gateway had a busy session.
const BUSY: Duration = Duration::from_secs(6);
/// A row when it had none.
const IDLE: Duration = Duration::from_secs(15);
const RUNS: u32 = 20;

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
fn held(name: &str, mut took: Vec<Duration>, bound: Duration) {
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
    assert!(over == 0, "{name}: {over} of {} over {bound:?}", took.len());
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
