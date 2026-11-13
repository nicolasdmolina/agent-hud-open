//! `Log::count_running_total`: first counts, dated growth, zero sightings
//! and the monotonic contract.
use super::{records::Running, Log, Usage};

const FIRST: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

fn log() -> Log {
    Log::new(std::env::temp_dir().join("state.db#s"), "hermes")
}

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input,
        output,
        ..Default::default()
    }
}

fn read(log: &mut Log, total: Usage, last_seen: Option<i64>, now: i64) {
    let running = Running { last_seen };
    log.count_running_total("k".into(), "m", FIRST, running, now, total, None);
}

fn meter(log: &Log) -> Vec<(i64, u64, u64)> {
    let mut events = log
        .meter_events
        .values()
        .map(|e| (e.at, e.usage.input, e.usage.output))
        .collect::<Vec<_>>();
    events.sort();
    events
}

#[test]
fn growth_is_dated_at_a_last_seen_that_moved_and_grouped_by_period() {
    let mut log = log();
    read(&mut log, usage(100, 10), Some(FIRST), FIRST + 60_000);
    read(
        &mut log,
        usage(300, 30),
        Some(FIRST + HOUR),
        FIRST + 3 * HOUR,
    );
    // Twice in one later period: one event, at the latest stamp.
    read(
        &mut log,
        usage(350, 35),
        Some(FIRST + 2 * HOUR),
        FIRST + 3 * HOUR,
    );
    read(
        &mut log,
        usage(400, 40),
        Some(FIRST + 2 * HOUR + 60_000),
        FIRST + 3 * HOUR,
    );
    assert_eq!(
        meter(&log),
        [
            (FIRST, 100, 10),
            (FIRST + HOUR, 200, 20),
            (FIRST + 2 * HOUR + 60_000, 100, 10)
        ]
    );
    assert_eq!((log.usage.input, log.usage.output), (400, 40));
}

#[test]
fn growth_under_a_stale_or_missing_last_seen_is_dated_at_the_read() {
    let mut log = log();
    read(&mut log, usage(100, 10), Some(FIRST), FIRST);
    // Grown, but the stamp has not moved: dated when this read saw it.
    read(&mut log, usage(200, 20), Some(FIRST), FIRST + HOUR);
    // Grown with no stamp at all.
    read(&mut log, usage(300, 30), None, FIRST + 2 * HOUR);
    // A stamp older than one already seen has not moved either.
    read(
        &mut log,
        usage(400, 40),
        Some(FIRST - HOUR),
        FIRST + 3 * HOUR,
    );
    assert_eq!(
        meter(&log),
        [
            (FIRST, 100, 10),
            (FIRST + HOUR, 100, 10),
            (FIRST + 2 * HOUR, 100, 10),
            (FIRST + 3 * HOUR, 100, 10)
        ]
    );
}

#[test]
fn a_row_first_read_at_zero_dates_its_first_tokens_as_growth() {
    let mut log = log();
    read(&mut log, usage(0, 0), Some(FIRST), FIRST);
    assert!(log.meter_events.is_empty(), "a zero row makes no event");
    read(
        &mut log,
        usage(100, 0),
        Some(FIRST + HOUR),
        FIRST + 2 * HOUR,
    );
    assert_eq!(meter(&log), [(FIRST + HOUR, 100, 0)]);
    // After a relaunch the log, and what it has seen, is read back.
    let mut log: Log = serde_json::from_str(&serde_json::to_string(&log).unwrap()).unwrap();
    read(
        &mut log,
        usage(100, 0),
        Some(FIRST + HOUR),
        FIRST + 3 * HOUR,
    );
    assert_eq!(log.usage.input, 100);
}

#[test]
fn a_total_is_cumulative_so_a_smaller_one_adds_nothing_until_passed() {
    let mut log = log();
    let mut totals = Vec::new();
    for (step, input) in [100, 50, 80, 120].into_iter().enumerate() {
        let at = FIRST + step as i64 * HOUR;
        read(&mut log, usage(input, 0), Some(at), at);
        totals.push(log.usage.input);
    }
    assert_eq!(totals, [100, 100, 100, 120]);
    assert_eq!(meter(&log), [(FIRST, 100, 0), (FIRST + 3 * HOUR, 20, 0)]);
}

#[test]
fn a_total_read_again_unchanged_adds_nothing() {
    let mut log = log();
    read(&mut log, usage(100, 10), Some(FIRST), FIRST);
    read(&mut log, usage(100, 10), Some(FIRST + HOUR), FIRST + HOUR);
    read(&mut log, usage(100, 10), None, FIRST + 2 * HOUR);
    assert_eq!(meter(&log), [(FIRST, 100, 10)]);
}
