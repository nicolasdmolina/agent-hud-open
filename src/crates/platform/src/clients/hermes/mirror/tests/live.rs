//! Counts through the writer: a far session's sample is kept in its
//! mirror and shows on its row; counts are committed, and core told, no
//! more often than the writer's spacing, however often they move; a
//! round whose counts did not move writes nothing; and a count read
//! under a route number that is no longer the machine's is dropped.
//!
//! A count is asked apart from the polls (ticket 60), so it reaches the
//! writer after the list that gave its session a row: a new session's
//! row is one commit, and its first count the next (`FIRST`).
use super::*;
use agent_hud_agents::hermes::gateway::Sample;

/// A spacing short enough to wait out, and long enough to see.
const SPACE: Duration = Duration::from_millis(400);

/// The commits a new session costs: its row, then its first count.
const FIRST: usize = 2;

fn sample(total: u64) -> Sample {
    Sample {
        input: total - total / 4,
        output: total / 4,
        cache_read: 0,
        cache_write: 0,
    }
}

/// What a round of count calls said of `profile`: each session with a
/// total over zero.
fn tallied(
    profile: &Profile,
    sessions: &[(&str, u64)],
) -> BTreeMap<Profile, Vec<(String, Sample)>> {
    let samples = sessions
        .iter()
        .filter(|(_, total)| *total > 0)
        .map(|(id, total)| ((*id).to_owned(), sample(*total)));
    BTreeMap::from([(profile.clone(), samples.collect())])
}

impl Bench {
    /// Queues a poll's listing of `profile` and what the round of count
    /// calls after it said each session has used, and waits until the
    /// writer has taken both.
    fn counts(&self, profile: &Profile, sessions: &[(&str, u64)], read_ms: i64, route: u64) {
        let rows: Vec<(&str, &str)> = sessions.iter().map(|(id, _)| (*id, "Counted")).collect();
        self.writer.listed(listing(profile, &rows, read_ms), route);
        self.writer.settle();
        self.writer
            .counted(tallied(profile, sessions), read_ms, route);
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
fn a_new_sessions_count_is_written_after_its_row_and_shows_on_it() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 120), ("s2", 0)], 1_000, 1);
    // One commit for the rows and one for the count, each told to core.
    assert_eq!(bench.tells(), FIRST);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 120)]);
    assert_eq!(bench.turn(), 1);
    assert_eq!(
        bench.tokens(),
        [
            ("default:s1".to_owned(), json!(120)),
            ("default:s2".to_owned(), json!(0))
        ]
    );
    let log = bench.log();
    assert!(log.contains("mirror wrote profile=default rows=2 counts=0"));
    assert!(log.contains("mirror wrote profile=default rows=0 counts=1"));
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
    assert_eq!(bench.tells(), FIRST);
    assert_eq!(stamps(&bench.file(&Profile::Default)), stamp);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 120)]);
}

/// A count that moves at every poll is committed once in each spacing,
/// and the newest one is the one that lands, also where no poll follows.
#[test]
fn counts_are_committed_no_more_often_than_the_spacing_and_the_newest_lands() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    assert_eq!(bench.tells(), FIRST);
    // Ten polls in a tenth of the spacing, the count growing in each.
    for step in 1..=10 {
        bench.counts(
            &Profile::Default,
            &[("s1", 100 + step * 10)],
            1_000 + step as i64,
            1,
        );
    }
    assert_eq!(bench.tells(), FIRST, "held until the spacing has passed");
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    // No further poll: the count that waits is written when its time
    // comes, and it is the newest.
    bench.told(FIRST + 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 200)]);
    let at = bench.told.at.lock().unwrap().clone();
    assert!(at[2] - at[1] >= SPACE, "{:?}", at[2] - at[1]);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), FIRST + 1, "one commit for the ten");
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
    bench.told(FIRST + 2);
    let at = bench.told.at.lock().unwrap().clone();
    // From the first commit of a count on.
    let stream = &at[FIRST..];
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
    // Two rows and two first counts, the second of which waits out the
    // spacing.
    bench.told(FIRST * 2);
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
    assert_eq!(bench.tells(), FIRST);
    // One that arrives late under the old number is dropped too, and
    // one read under the new number is written.
    bench.counts(&Profile::Default, &[("s1", 170)], 3_000, 1);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    bench.counts(&Profile::Default, &[("s1", 180)], 2_500, 2);
    bench.told(FIRST + 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 180)]);
}

/// A count that arrives late, read under the old route number, does not
/// take the place of one read under the machine's number now that
/// waits for its time: it is dropped when it is taken, and the one
/// that waits is written.
#[test]
fn a_late_count_of_an_old_route_number_leaves_the_one_that_waits() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.number.now.store(2, Ordering::Release);
    bench.counts(&Profile::Default, &[("s1", 180)], 2_000, 2);
    // Read earlier, under the first number, and more than any.
    bench
        .writer
        .counted(tallied(&Profile::Default, &[("s1", 900)]), 1_500, 1);
    bench.writer.settle();
    bench.told(FIRST + 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 180)]);
    std::thread::sleep(SPACE * 2);
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

/// A count that says no more than its mirror is known to hold does not
/// wait to be written (ticket 35's verifier, v06): no write is due for
/// it, so the writer opens none. One that says more waits, alone.
#[test]
fn a_count_the_mirror_is_known_to_hold_does_not_wait_to_be_written() {
    use super::super::counts::Counts;
    let mut counts = Counts::default();
    let one = |id: &str, total| vec![(id.to_owned(), sample(total))];
    counts.sample(&Profile::Default, one("s1", 100), 1_000, 1);
    let (_, waited) = counts.take(&Profile::Default).unwrap();
    counts.said(&Profile::Default, &waited);
    counts.counted();
    // The same again at every round, and less: nothing waits.
    for (read_ms, total) in [(2_000, 100), (3_000, 100), (4_000, 60)] {
        counts.sample(&Profile::Default, one("s1", total), read_ms, 1);
        assert!(counts.waiting().is_empty(), "{total} waits");
        assert!(counts.due(SPACE).is_none(), "a write is due for {total}");
    }
    // Among several, only the one that says more waits.
    let mut round = one("s1", 100);
    round.extend(one("s2", 40));
    counts.sample(&Profile::Default, round, 5_000, 1);
    let (_, waited) = counts.take(&Profile::Default).unwrap();
    assert_eq!(waited, vec![("s2".to_owned(), sample(40), 5_000)]);
}

/// A row written while a count waits for its time leaves the count
/// waiting: it is written when its time comes, with no further poll.
#[test]
fn a_row_written_before_a_counts_time_leaves_the_count_waiting() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    assert_eq!(bench.tells(), FIRST);
    // A new session is listed: its row is written at once.
    bench.counts(&Profile::Default, &[("s1", 150), ("s2", 0)], 3_000, 1);
    assert_eq!(bench.tells(), FIRST + 1);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    bench.told(FIRST + 2);
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
    bench.told(FIRST + 1);
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
    // The list asks when it is taken and has nothing to write. The
    // count asks when it is taken, and again to commit.
    bench.number.raise_at.store(asked + 3, Ordering::Release);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.number.now.load(Ordering::Acquire), 2);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    assert_eq!(bench.tells(), FIRST);
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
    again.listed(listing(&Profile::Default, &[("s1", "Counted")], 5_000), 1);
    again.counted(tallied(&Profile::Default, &[("s1", 100)]), 5_000, 1);
    again.settle();
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), tells);
    let after = std::fs::metadata(bench.file(&Profile::Default)).unwrap();
    assert_eq!((after.len(), after.modified().unwrap()), stamp);
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
    again.stop();
}

/// The verifier's probe of ticket 60 as a test (ticket 64): a count
/// that reaches the writer before the list that gives its session a
/// row. `hud_live` keeps no count of a session without a row, so the
/// count is held, and it is written once the list names the session,
/// with no further round: 300 was the session's last count. The rounds
/// that say it again write nothing more.
#[test]
fn a_count_taken_before_its_row_shows_once_the_row_is_listed() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    // The mirror is there, with another session's row and count.
    bench.counts(&Profile::Default, &[("s0", 50)], 1_000, 1);
    std::thread::sleep(SPACE * 2);
    // s1's count is taken before its row is written.
    bench
        .writer
        .counted(tallied(&Profile::Default, &[("s1", 300)]), 2_000, 1);
    bench.writer.settle();
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.kept(&Profile::Default), [("s0".to_owned(), 50)]);
    let tells = bench.tells();
    // Its row, and no round after it.
    let rows = [("s0", "Counted"), ("s1", "Counted")];
    bench.lists(&Profile::Default, &rows, 3_000, 1);
    // The row and the count are one commit: the count's time has come.
    bench.told(tells + 1);
    assert_eq!(
        bench.kept(&Profile::Default),
        [("s0".to_owned(), 50), ("s1".to_owned(), 300)]
    );
    bench.turn();
    assert_eq!(
        bench.tokens(),
        [
            ("default:s0".to_owned(), json!(50)),
            ("default:s1".to_owned(), json!(300))
        ]
    );
    // Three more rounds saying the same: nothing more is written.
    let tells = bench.tells();
    for round in 0..3 {
        bench.counts(
            &Profile::Default,
            &[("s0", 50), ("s1", 300)],
            4_000 + round,
            1,
        );
    }
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.tells(), tells);
}

/// The same where the profile has no mirror yet: a count alone makes
/// none, and is written with the first row the list gives.
#[test]
fn a_count_taken_before_its_mirror_is_made_shows_once_the_row_is_listed() {
    let mut bench = bench_spaced("Mac mini", SPACE);
    bench
        .writer
        .counted(tallied(&reports(), &[("r1", 70)]), 1_000, 1);
    bench.writer.settle();
    assert!(!bench.file(&reports()).exists(), "a count made a mirror");
    bench.lists(&reports(), &[("r1", "Counted")], 2_000, 1);
    bench.told(2);
    assert_eq!(bench.kept(&reports()), [("r1".to_owned(), 70)]);
    bench.turn();
    assert_eq!(bench.tokens(), [("reports:r1".to_owned(), json!(70))]);
}

/// A count held for want of its row is not held for ever: a list read
/// after it that does not name its session drops it, and so does a
/// list read under another route number.
#[test]
fn a_count_held_for_its_row_is_dropped_by_a_newer_list_that_leaves_it_out() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s0", 50)], 1_000, 1);
    std::thread::sleep(SPACE * 2);
    let held = |bench: &Bench, id: &str, read_ms| {
        let counted = tallied(&Profile::Default, &[(id, 300)]);
        bench.writer.counted(counted, read_ms, 1);
        bench.writer.settle();
    };
    // Left out by a list read after it.
    held(&bench, "s1", 2_000);
    bench.lists(&Profile::Default, &[("s0", "Counted")], 3_000, 1);
    bench.lists(
        &Profile::Default,
        &[("s0", "Counted"), ("s1", "Back")],
        4_000,
        1,
    );
    // Held, then the route number goes up.
    held(&bench, "s2", 5_000);
    // Its time comes, and it is held for want of its row.
    std::thread::sleep(SPACE * 2);
    bench.number.now.store(2, Ordering::Release);
    let rows = [("s0", "Counted"), ("s1", "Back"), ("s2", "New")];
    bench.lists(&Profile::Default, &rows, 6_000, 2);
    std::thread::sleep(SPACE * 2);
    assert_eq!(bench.kept(&Profile::Default), [("s0".to_owned(), 50)]);
}

/// Counts that waited under the old route number wait no more once a
/// list read under the new one is taken (ticket 64, the verifier's x07
/// of ticket 60): no commit of them is tried when their time comes.
#[test]
fn a_list_under_a_new_route_number_ends_the_wait_of_old_counts() {
    let bench = bench_spaced("Mac mini", SPACE);
    bench.counts(&Profile::Default, &[("s1", 100)], 1_000, 1);
    bench.counts(&Profile::Default, &[("s1", 150)], 2_000, 1);
    bench.number.now.store(2, Ordering::Release);
    bench.lists(&Profile::Default, &[("s1", "Counted")], 3_000, 2);
    std::thread::sleep(SPACE * 3);
    let log = bench.log();
    assert!(!log.contains("why=route"), "{log}");
    assert_eq!(bench.kept(&Profile::Default), [("s1".to_owned(), 100)]);
}
