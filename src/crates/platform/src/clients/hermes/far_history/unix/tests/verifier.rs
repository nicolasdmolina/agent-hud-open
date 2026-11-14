//! The probes of the first review of this reader
//! (`evidence/36/verifier-report.md`, findings 1 and 2), kept as tests.
//! Finding 3's is `v_completion_counts_are_checked_for_every_kind`, beside
//! the answer's own tests in the agents crate.
use super::super::{Hold, Settings, Slots};
use super::schema;
use super::world::{kill, open, raw, readers, sessions, settings, World};
use crate::ssh_link::unix::tests::lab::until;
use crate::ssh_link::{End, Link, Ran};
use agent_hud_agents::hermes::far_history::{self as far, Reply, Request};
use rusqlite::Connection;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

/// The most memory, in KiB, of a reader whose every long value is one
/// SQLite can measure without loading: 40 MiB. This one is a bound. A
/// reader that only starts and reads a small row holds about 22 MiB over
/// its whole life here, and one that walks past a long value it does not
/// load holds about 31, the page cache being the difference.
const MOST_MEMORY: u64 = 40 * 1024;
/// The most memory, in KiB, of a reader that its own watch had to end,
/// over a long value in a column declared REAL: 64 MiB. This one is not a
/// bound. The watch looks every half millisecond and the process is ended
/// at the next look, so its memory passes the mark (about 36.6 MiB here)
/// by what arrives until then. Measured over 110 readers on a quiet Mac:
/// 39.7 MiB at most. The second review measured 46 MiB with the watch at
/// 2 ms. The ceiling leaves room for a look that comes a few ms late.
const MOST_WATCHED_MEMORY: u64 = 64 * 1024;

/// One machine, two profiles. A reader of the first is still running
/// when the machine is retired and added again, and the next read is of
/// the second. The slot knows nothing by then, and the probe it makes
/// asked only about the second home, found nobody, and let a second
/// reader start beside the first.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn v_readded_machine_must_exclude_an_old_reader_of_another_profile() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let second = world.lab.at("second-home");
    fs::create_dir_all(&second).unwrap();
    let second = fs::canonicalize(second).unwrap().display().to_string();
    let other = Connection::open(format!("{second}/state.db")).unwrap();
    other.execute_batch(schema::HERMES).unwrap();
    other
        .execute_batch(
            "PRAGMA journal_mode=WAL; DROP TABLE sessions;
             CREATE VIEW sessions AS WITH RECURSIVE spin(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM spin)
             SELECT n AS rowid, 'endless' AS id, 1.0 AS started_at FROM spin WHERE n < 0;",
        )
        .unwrap();
    let long = Settings {
        deadline: Duration::from_secs(10),
        ..settings()
    };
    let (reader, slots, link) = (world.reader(long), Slots::new(), world.link());
    let key = world.machine();
    let slot = slots.of(&key);
    let request = sessions(&["endless"]);
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let (first_database, second_database) = (
        world.database().display().to_string(),
        format!("{second}/state.db"),
    );
    // One machine, so one start of the mark, and a mark for each home.
    let second_mark = far::mark(&key, &second);
    assert_ne!(second_mark, world.mark());
    let mut most_open = 0;
    let mut trace = Vec::new();
    let mut answers = Vec::new();
    thread::scope(|scope| {
        let first = scope.spawn(|| reader.read(&link, &slot, &world.home, &request));
        assert!(until(Duration::from_secs(10), || {
            !open(&readers(&world.mark()), &first_database).is_empty()
        }));
        slots.retired(&key);
        link.close();
        let ended = first.join().unwrap();
        println!("  first caller ended={ended:?}, hold={:?}", slot.hold());
        assert_eq!(slot.hold(), Hold::StartUnknown);
        let again = world.link();
        let short = world.reader(Settings {
            deadline: Duration::from_secs(2),
            ..settings()
        });
        let began = Instant::now();
        // The second profile is asked for at once, and again and again
        // until the first one's reader is past its deadline.
        while began.elapsed() < Duration::from_secs(14) {
            thread::scope(|inner| {
                let next = inner.spawn(|| short.read(&again, &slot, &second, &request));
                while !next.is_finished() {
                    let a = open(&readers(&world.mark()), &first_database);
                    let b = open(&readers(&second_mark), &second_database);
                    most_open = most_open.max(a.len() + b.len());
                    if a.len() + b.len() > 1 && trace.is_empty() {
                        trace.push(format!(
                            "home A open by {a:?}; home B open by {b:?}; same machine slot"
                        ));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                answers.push((began.elapsed().as_millis(), next.join().unwrap()));
            });
            thread::sleep(Duration::from_millis(150));
        }
        again.close();
    });
    // Every far reader has ended before anything is asserted.
    assert!(until(Duration::from_secs(15), || {
        readers(&world.mark()).is_empty() && readers(&second_mark).is_empty()
    }));
    for (at, answer) in &answers {
        println!("  {at} ms: {answer:?}");
    }
    println!("  maximum simultaneous far readers with their database open={most_open}; trace={trace:?}; final orphan count=0");
    assert!(
        most_open <= 1,
        "one machine's readers overlapped across homes after retirement/re-add"
    );
    // The second profile's first read was told a reader is alive, and it
    // was read once that reader had gone.
    assert_eq!(answers[0].1, Err(super::super::Failure::Busy));
    assert_eq!(
        answers.last().unwrap().1,
        Err(super::super::Failure::Exit(3))
    );
}

/// Runs the program once and takes the far reader's highest memory over
/// its whole life, as the system counted it (`World::measure_memory`).
fn peak_read(world: &World, link: &Link, ask: &Request, deadline: u64) -> (Ran, u64, Duration) {
    let began = Instant::now();
    let before = world.peaks().len();
    let ran = thread::scope(|scope| {
        let job = scope.spawn(|| raw(link, &world.home, Duration::from_secs(deadline), ask));
        while !job.is_finished() {
            // A watchdog of the test's own, apart from the reader's.
            if began.elapsed() > Duration::from_secs(deadline + 8) {
                for pid in readers(&world.mark()) {
                    kill(pid);
                }
                panic!("external watchdog expired");
            }
            thread::sleep(Duration::from_millis(10));
        }
        job.join().unwrap()
    });
    let peaks = world.peaks();
    assert_eq!(
        peaks.len(),
        before + 1,
        "one reader ran, and its memory was written down"
    );
    (ran, peaks[before], began.elapsed())
}

/// A database of 400 MB, and in it one session whose title is 8 MiB and
/// then 64 MiB. The output bound is 1 MiB. The reader built each whole
/// row and its JSON before it looked at the bound, so the title's length
/// was the reader's memory: about 270 MB.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn v_large_database_has_bounded_memory_and_deadline() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    world.add_session(&db, "s1", 2);
    world.add_session(&db, "s2", 2);
    world.measure_memory();
    db.execute_batch("CREATE TABLE payload(bytes BLOB); BEGIN;")
        .unwrap();
    for _ in 0..96 {
        db.execute("INSERT INTO payload VALUES(zeroblob(4194304))", [])
            .unwrap();
    }
    db.execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let size = fs::metadata(world.database()).unwrap().len();
    assert!(size > 380 * 1024 * 1024);
    let link = world.link();
    let ask = sessions(&["s1"]);
    let (ran, normal, took) = peak_read(&world, &link, &ask, 10);
    assert_eq!(ran.end, End::Ended(0));
    assert!(matches!(
        far::parse(&ask, &ran.stdout),
        Ok(Reply::Sessions(_))
    ));
    println!(
        "  large db={size} bytes; normal read={} bytes, peak_rss={normal} KiB, elapsed={took:?}",
        ran.stdout.len()
    );
    let page = Request::Prints {
        after: 0,
        limit: 100,
        since: None,
    };
    let mut peaks = vec![normal];
    let mut watched = Vec::new();
    for megabytes in [8, 64] {
        db.execute(
            "UPDATE sessions SET title=replace(hex(zeroblob(?1)),'0','x') WHERE id='s1'",
            [megabytes * 1024 * 1024 / 2],
        )
        .unwrap();
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        let (ran, rss, took) = peak_read(&world, &link, &ask, 20);
        println!(
            "  title={megabytes} MiB; requested output bound={} bytes; result={:?}; returned={} bytes; peak_rss={rss} KiB; elapsed={took:?}",
            far::MOST_OUTPUT,
            ran.end,
            ran.stdout.len()
        );
        assert_eq!(ran.end, End::Ended(0));
        // The session is too large, and is said to be: nothing of it came.
        assert_eq!(far::parse(&ask, &ran.stdout), Ok(Reply::TooLarge));
        peaks.push(rss);
        // The session beside it is read as ever, and so is the page of
        // fingerprints, which has both.
        let beside = sessions(&["s2"]);
        let (ran, rss, _) = peak_read(&world, &link, &beside, 20);
        assert!(
            matches!(far::parse(&beside, &ran.stdout), Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1)
        );
        peaks.push(rss);
        let (ran, rss, _) = peak_read(&world, &link, &page, 20);
        let Ok(Reply::Prints(prints)) = far::parse(&page, &ran.stdout) else {
            panic!("not a page");
        };
        assert_eq!(prints.prints.len(), 2);
        peaks.push(rss);
    }
    // A title that begins with a NUL, which ends a text for SQLite's
    // count of characters and not for its count of bytes, and one of
    // 40,000 characters of four bytes each: 160,000 bytes. Each is over
    // 64 KiB, and neither is loaded.
    for (what, title) in [
        (
            "16 MiB after a NUL",
            "char(0)||replace(hex(zeroblob(8388608)),'0','x')",
        ),
        (
            "160,000 bytes in 40,000 characters",
            "replace(hex(zeroblob(20000)),'0',char(128512))",
        ),
        ("a 64 MiB blob", "zeroblob(67108864)"),
    ] {
        db.execute_batch(&format!(
            "UPDATE sessions SET title={title} WHERE id='s1'; PRAGMA wal_checkpoint(TRUNCATE)"
        ))
        .unwrap();
        let (ran, rss, _) = peak_read(&world, &link, &ask, 20);
        println!(
            "  title of {what}: result={:?}; returned={} bytes; peak_rss={rss} KiB",
            ran.end,
            ran.stdout.len()
        );
        assert_eq!(ran.end, End::Ended(0));
        assert_eq!(far::parse(&ask, &ran.stdout), Ok(Reply::TooLarge), "{what}");
        peaks.push(rss);
    }
    // A count the fingerprint is made of, as long: the fingerprint says
    // so, and is no session's own. The session read by id is too large.
    db.execute(
        "UPDATE session_model_usage SET input_tokens=replace(hex(zeroblob(?1)),'0','x') WHERE session_id='s2' AND task=''",
        [32 * 1024 * 1024],
    )
    .unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let (ran, rss, took) = peak_read(&world, &link, &page, 20);
    let Ok(Reply::Prints(prints)) = far::parse(&page, &ran.stdout) else {
        panic!("not a page");
    };
    println!(
        "  a 64 MiB usage count: fingerprint page of {} bytes, peak_rss={rss} KiB, elapsed={took:?}",
        ran.stdout.len()
    );
    assert_eq!(prints.prints[1].print, "oversized");
    assert_ne!(prints.prints[0].print, "oversized");
    peaks.push(rss);
    let (ran, rss, _) = peak_read(&world, &link, &sessions(&["s2"]), 20);
    assert_eq!(
        far::parse(&sessions(&["s2"]), &ran.stdout),
        Ok(Reply::TooLarge)
    );
    peaks.push(rss);

    // A stamp, as long. Its column is declared REAL, and of such a column
    // SQLite loads a value even to say how long it is. So the reader's
    // own watch on its memory ends it, with its own status, before the
    // value is all there.
    db.execute(
        "UPDATE session_model_usage SET input_tokens=5, last_seen=replace(hex(zeroblob(?1)),'0','x') WHERE session_id='s2' AND task=''",
        [128 * 1024 * 1024],
    )
    .unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    for ask in [page.clone(), sessions(&["s2"])] {
        let (ran, rss, took) = peak_read(&world, &link, &ask, 20);
        println!(
            "  a 256 MiB usage stamp, {}: result={:?}; returned={} bytes; peak_rss={rss} KiB; elapsed={took:?}",
            ask.kind(),
            ran.end,
            ran.stdout.len()
        );
        assert_eq!(ran.end, End::Ended(far::TOO_MUCH_MEMORY));
        assert_eq!(
            far::parse(&ask, &ran.stdout),
            Err(far::Rejected::NoCompletion)
        );
        assert!(readers(&world.mark()).is_empty());
        watched.push(rss);
    }
    // The caller takes that status as it takes "too large": the session
    // is skipped whole, and the one beside it is read.
    let (reader, slots) = (world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let read = reader.sessions(
        &link,
        &slot,
        &world.home,
        &["s1".to_owned(), "s2".to_owned()],
    );
    println!(
        "  through the caller: read {}, skipped {:?}, stopped {:?}",
        read.read.len(),
        read.skipped,
        read.stopped
    );
    assert_eq!(read.stopped, None);
    assert_eq!(read.skipped.len(), 2);
    assert!(read
        .skipped
        .iter()
        .all(|(_, why)| *why == super::super::Skip::TooLarge));
    db.execute("UPDATE sessions SET title='short again' WHERE id='s1'", [])
        .unwrap();
    let read = reader.sessions(
        &link,
        &slot,
        &world.home,
        &["s1".to_owned(), "s2".to_owned()],
    );
    assert_eq!(
        (read.read.len(), read.skipped.len(), read.stopped),
        (1, 1, None)
    );

    world.make_every_read_endless();
    let (ran, rss, took) = peak_read(&world, &link, &sessions(&["endless"]), 2);
    println!(
        "  large-db endless query: exit={:?}; peak_rss={rss} KiB; elapsed={took:?}; remaining readers={:?}",
        ran.end,
        readers(&world.mark())
    );
    assert_eq!(ran.end, End::Ended(3));
    assert!(took < Duration::from_secs(5));
    assert!(readers(&world.mark()).is_empty());
    link.close();
    drop(db);
    // Every reader of this test, the ones the caller started among them,
    // over its whole life.
    let all = world.peaks();
    let (least, most) = (all.iter().min().unwrap(), all.iter().max().unwrap());
    println!("  readers whose long values were never loaded: {peaks:?} KiB; the most allowed is {MOST_MEMORY} KiB");
    println!("  readers their own watch ended: {watched:?} KiB");
    println!(
        "  all {} readers: from {least} to {most} KiB over their whole lives; the most allowed is {MOST_WATCHED_MEMORY} KiB",
        all.len()
    );
    assert!(
        peaks.iter().all(|rss| *rss < MOST_MEMORY),
        "1 MiB output bound permits database-controlled RSS growth: {peaks:?} KiB"
    );
    assert!(
        all.iter().all(|rss| *rss < MOST_WATCHED_MEMORY),
        "the memory watch let a reader grow too far: {all:?} KiB"
    );
    assert!(
        all.iter().all(|rss| *rss > 4 * 1024),
        "the reader's memory was not measured: {all:?}"
    );
    assert_eq!(watched.len(), 2);
}

/// The probe of the second review (`evidence/36/verifier-report-2.md`,
/// finding 1). Three sessions in rowid order, and the middle one has
/// 256 MiB of text where a stamp belongs. The far reader ends itself on
/// that row every time, and the page it is in came back as "too large"
/// with nothing to go on by, so a sweep stopped there for good.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn poison_stamp_must_not_stop_the_whole_history_sweep() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    for id in ["good-before", "poison", "good-after"] {
        world.add_session(&db, id, 1);
    }
    db.execute(
        "UPDATE sessions SET last_activity_at=replace(hex(zeroblob(?1)),'0','x') WHERE id='poison'",
        [128 * 1024 * 1024],
    )
    .unwrap();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    // A whole sweep, a page at a time, each from where the last ended.
    for since in [None, Some(1.0)] {
        let (mut cursor, mut found, mut skipped, mut pages) = (0, Vec::new(), Vec::new(), 0);
        loop {
            let page = reader
                .prints(&link, &slot, &world.home, cursor, 100, since)
                .unwrap();
            pages += 1;
            println!(
                "  page {pages}, cursor {cursor} => last={}, seen={}, ids={:?}, skipped={:?}, ended={}",
                page.last,
                page.seen,
                page.prints.iter().map(|print| &print.id).collect::<Vec<_>>(),
                page.skipped,
                page.ended
            );
            assert!(
                page.last > cursor || page.ended,
                "the sweep did not move on from {cursor}"
            );
            cursor = page.last;
            found.extend(page.prints.into_iter().map(|print| print.id));
            skipped.extend(page.skipped);
            if page.ended {
                break;
            }
            assert!(pages < 20, "the sweep never ended");
        }
        // The sessions on either side were read, and the one between
        // them is named by its rowid and nothing else.
        assert_eq!(found, ["good-before", "good-after"]);
        assert_eq!(skipped, [2]);
    }
    let log = world.logged();
    let said: Vec<&String> = log
        .iter()
        .filter(|line| line.contains("quarantines"))
        .collect();
    assert_eq!(said.len(), 2);
    assert_eq!(
        *said[0],
        "history prints quarantines rowid 2: too large to read, and stepped over"
    );
    // Read by id, the whole session is still refused, and the two beside
    // it are read.
    let ids: Vec<String> = ["good-before", "poison", "good-after"]
        .map(str::to_owned)
        .to_vec();
    let read = reader.sessions(&link, &slot, &world.home, &ids);
    assert_eq!(read.stopped, None);
    let came: Vec<&str> = read
        .read
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(came, ["good-before", "good-after"]);
    assert_eq!(
        read.skipped,
        [("poison".to_owned(), super::super::Skip::TooLarge)]
    );
    // A table that ends on such a row ends the sweep after it.
    db.execute("DELETE FROM sessions WHERE id='good-after'", [])
        .unwrap();
    let page = reader
        .prints(&link, &slot, &world.home, 1, 100, None)
        .unwrap();
    assert_eq!(
        (
            page.prints.len(),
            page.skipped.as_slice(),
            page.last,
            page.ended
        ),
        (0, &[2][..], 2, false)
    );
    let page = reader
        .prints(&link, &slot, &world.home, 2, 100, None)
        .unwrap();
    assert_eq!(
        (page.prints.len(), page.skipped.len(), page.last, page.ended),
        (0, 0, 2, true)
    );
    link.close();
    assert!(readers(&world.mark()).is_empty());
}
