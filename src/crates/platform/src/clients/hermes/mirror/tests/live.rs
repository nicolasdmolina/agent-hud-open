//! Counts through the writer: a far session's sample is kept in its
//! mirror and shows on its row; counts are committed, and core told, no
//! more often than the writer's spacing, however often they move; a
//! poll whose counts did not move writes nothing; and a count read
//! under a route number that is no longer the machine's is dropped.
use super::*;
use agent_hud_agents::hermes::gateway::Sample;

/// A spacing short enough to wait out, and long enough to see.
const SPACE: Duration = Duration::from_millis(400);

fn sample(total: u64) -> Sample {
    Sample {
        input: total - total / 4,
        output: total / 4,
        cache_read: 0,
        cache_write: 0,
    }
}

impl Bench {
    /// Queues a poll's listing of `profile` with what its samples said
    /// each session has used, and waits until the writer has taken it.
    fn counts(&self, profile: &Profile, sessions: &[(&str, u64)], read_ms: i64, route: u64) {
        let rows: Vec<(&str, &str)> = sessions.iter().map(|(id, _)| (*id, "Counted")).collect();
        let mut listing = listing(profile, &rows, read_ms);
        let samples = sessions
            .iter()
            .filter(|(_, total)| *total > 0)
            .map(|(id, total)| ((*id).to_owned(), sample(*total)));
        listing.samples.insert(profile.clone(), samples.collect());
        self.writer.listed(listing, route);
        self.writer.settle();
    }

    /// What `hud_live` of the mirror of `profile` holds: session, total.
    fn kept(&self, profile: &Profile) -> Vec<(String, i64)> {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let db = rusqlite::Connection::open_with_flags(self.file(profile), flags).unwrap();
        let sql =
            "SELECT session_id, input_tokens+output_tokens+cache_read_tokens+cache_write_tokens
            FROM hud_live ORDER BY session_id";
        let mut statement = db.prepare(sql).unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)));
        rows.unwrap().map(Result::unwrap).collect()
    }

    /// Waits until the writer has told core `count` times.
    fn told(&self, count: usize) {
        let began = Instant::now();
        while self.tells() < count {
            assert!(
                began.elapsed() < Duration::from_secs(10),
                "{} tells",
                self.tells()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Each far session's id and tokens, as core lists them.
    fn tokens(&mut self) -> Vec<(String, Value)> {
        (self.rows().into_iter())
            .map(|(id, _, _, tokens)| (id, tokens))
            .collect()
    }
}

#[test]
fn a_new_sessions_count_is_written_with_its_row_and_shows_on_it() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 120), ("s2", 0)], 1_000, 1);
    // One commit for the rows and the count, and one word to core.
    assert_eq!(bench.tells(), 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 120)]);
    assert_eq!(bench.turn(), 1);
    assert_eq!(
        bench.tokens(),
        [
            ("default:s1".to_owned(), json!(120)),
            ("default:s2".to_owned(), json!(0))
        ]
    );
    assert!(bench
        .log()
        .contains("mirror wrote profile=default rows=2 counts=1"));
}

#[test]
fn a_poll_whose_counts_did_not_move_writes_nothing_and_tells_nothing() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 120)], 1_000, 1);
    let stamp = stamps(&bench.file(&Profile::Default));
    // The same count at every poll, a lower one, and none at all.
    for read_ms in 1..40 {
        let said = [120, 120, 90, 0][read_ms as usize % 4];
        bench.counts(&Profile::Default, &[("s1", said)], 1_000 + read_ms, 1);
    }
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), 1);
    assert_eq!(stamps(&bench.file(&Profile::Default)), stamp);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 120)]);
}

/// A count that moves at every poll is committed once in each spacing,
/// and the newest one is the one that lands, also where no poll follows.
#[test]
fn counts_are_committed_no_more_often_than_the_spacing_and_the_newest_lands() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    assert_eq!(bench.tells(), 1);
    // Ten polls in a tenth of the spacing, the count growing in each.
    for step in 1..=10 {
        bench.counts(
            &Profile::Default,
            &[("s1", 100 + step * 10)],
            1_000 + step as i64,
            1,
        );
    }
    assert_eq!(bench.tells(), 1, "held until the spacing has passed");
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    // No further poll: the count that waits is written when its time
    // comes, and it is the newest.
    bench.told(2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 200)]);
    let at = bench.told.at.lock().unwrap().clone();
    assert!(at[1] - at[0] >= SPACE, "{:?}", at[1] - at[0]);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), 2, "one commit for the ten");
    bench.turn();
    assert_eq!(bench.tokens(), [("default:s1".to_owned(), json!(200))]);

    // A steady stream: a count that grows every 20 ms for five spacings
    // is committed about five times, never sooner than the spacing.
    let began = Instant::now();
    let mut total = 200;
    while began.elapsed() < SPACE * 5 {
        total += 1;
        bench.counts(&Profile::Default, &[("s1", total)], 2_000 + total as i64, 1);
        std::thread::sleep(Duration::from_millis(20));
    }
    bench.told(3);
    let at = bench.told.at.lock().unwrap().clone();
    let stream = &at[1..];
    assert!((4..=8).contains(&stream.len()), "{} commits", stream.len());
    for pair in stream.windows(2) {
        assert!(pair[1] - pair[0] >= SPACE, "{:?}", pair[1] - pair[0]);
    }
}

/// The counts of two profiles that wait are written at one time, and
/// core is told of both files in one word.
#[test]
fn the_counts_of_every_profile_are_told_in_one_word() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&reports(), &[("r1", 50)], 1_000, 1);
    bench.told(2);
    bench.turn();
    bench.counts(&Profile::Default, &[("s1", 300)], 2_000, 1);
    bench.counts(&reports(), &[("r1", 70)], 2_000, 1);
    let tells = bench.tells();
    bench.told(tells + 1);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), tells + 1);
    assert_eq!(bench.turn(), 2, "both files in one word");
    assert_eq!(
        bench.tokens(),
        [
            ("default:s1".to_owned(), json!(300)),
            ("reports:r1".to_owned(), json!(70))
        ]
    );
}

#[test]
fn a_count_read_under_a_route_number_that_is_not_current_is_dropped() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    // A count that waits when the route changes was read under the old
    // number: it is not written when its time comes.
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    bench.number.now.store(2, Ordering::Release);
    std::thread::sleep(SPACE * 3);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    assert_eq!(bench.tells(), 1);
    // One that arrives late under the old number is dropped too, and
    // one read under the new number is written.
    bench.counts(&Profile::Default, &[("s1", 170)], 3_000, 1);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    bench.counts(&Profile::Default, &[("s1", 180)], 2_500, 2);
    bench.told(2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 180)]);
}

/// Counts that waited under the old route number are dropped when a
/// list read under the new one arrives, and do not take its rows with
/// them: the rows are written under their own number.
#[test]
fn counts_that_waited_under_an_old_route_do_not_take_new_rows_with_them() {
    let mut bench = bench_spaced("Mac mini", Duration::from_secs(30));
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    let tells = bench.tells();
    bench.number.now.store(2, Ordering::Release);
    bench.lists(
        &Profile::Default,
        &[("s1", "Counted"), ("s2", "New")],
        3_000,
        2,
    );
    assert_eq!(bench.tells(), tells + 1, "the new row is written");
    bench.turn();
    assert_eq!(bench.ids(), ["default:s1", "default:s2"]);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
}

/// The writer's own note of what waits (`Counts`): samples that waited
/// under another route number are gone the moment a list read under the
/// machine's number now is taken, whether or not that list brings a
/// sample. Through the writer this shows only in the moment a count's
/// time comes just as such a list is taken, since a count of an old
/// number is dropped at its commit all the same; so it is held here.
#[test]
fn samples_that_waited_under_another_route_number_wait_no_more() {
    use super::super::counts::Counts;
    let mut files = Counts::default();
    let one = |total| vec![("s1".to_owned(), sample(total))];
    files.sample(&Profile::Default, one(100), 1_000, 1);
    files.sample(&reports(), one(40), 1_000, 1);
    assert_eq!(files.waiting(), [Profile::Default, reports()]);
    // A list under the new number with no sample at all.
    files.sample(&Profile::Default, Vec::new(), 2_000, 2);
    assert_eq!(files.waiting(), [reports()]);
    assert!(files.take(&Profile::Default).is_none());
    // One with a sample: only that one waits, under the new number.
    files.sample(&reports(), one(70), 2_000, 2);
    let (route, waited) = files.take(&reports()).unwrap();
    assert_eq!(
        (route, waited),
        (2, vec![("s1".to_owned(), sample(70), 2_000)])
    );
    assert!(files.due(SPACE).is_none(), "nothing waits");
}

/// A row written while a count waits for its time leaves the count
/// waiting: it is written when its time comes, with no further poll.
#[test]
fn a_row_written_before_a_counts_time_leaves_the_count_waiting() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    assert_eq!(bench.tells(), 1);
    // A new session is listed: its row is written at once.
    bench.counts(&Profile::Default, &[("s1", 150), ("s2", 0)], 3_000, 1);
    assert_eq!(bench.tells(), 2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    bench.told(3);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 150)]);
}

/// Of two counts of one session that wait, the larger is written: a
/// count read later that says less (an agent Hermes loaded anew) does
/// not take the place of one that says more.
#[test]
fn of_two_counts_that_wait_the_larger_is_written() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 200)], 2_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 3_000, 1);
    bench.told(2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 200)]);
}

/// The number goes up after the writer looked at it and before it
/// commits a count: nothing is written.
#[test]
fn a_route_that_changes_before_a_count_is_committed_leaves_it_unwritten() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    std::thread::sleep(SPACE * 2);
    let asked = bench.number.asked.load(Ordering::Acquire);
    // The next write asks when it is taken, and again to commit.
    bench.number.raise_at.store(asked + 2, Ordering::Release);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.number.now.load(Ordering::Acquire), 2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    assert_eq!(bench.tells(), 1);
}

#[test]
fn a_writer_that_was_stopped_writes_no_count_that_waited() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    bench.writer.stop();
    let tells = bench.tells();
    std::thread::sleep(SPACE * 3);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    assert_eq!(bench.tells(), tells);
}

/// A writer made anew for a machine whose mirror already holds a count
/// (a relaunch) writes nothing for a poll that says the same.
#[test]
fn a_count_the_mirror_already_holds_is_not_written_again_after_a_relaunch() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.writer.stop();
    let again = writer_spaced(&bench.data, "Mac mini", &bench.told, &bench.number, SPACE);
    again.settle();
    let stamp = std::fs::metadata(bench.file(&Profile::Default)).unwrap();
    let (stamp, tells) = ((stamp.len(), stamp.modified().unwrap()), bench.tells());
    let mut said = listing(&Profile::Default, &[("s1", "Counted")], 5_000);
    said.samples
        .insert(Profile::Default, vec![("s1".to_owned(), sample(100))]);
    again.listed(said, 1);
    again.settle();
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), tells);
    let after = std::fs::metadata(bench.file(&Profile::Default)).unwrap();
    assert_eq!((after.len(), after.modified().unwrap()), stamp);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    again.stop();
}
