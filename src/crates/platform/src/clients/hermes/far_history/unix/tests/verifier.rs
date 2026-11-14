//! The probes of the first review of this reader
//! (`evidence/36/verifier-report.md`, findings 1 and 2), kept as tests.
//! Finding 3's is `v_completion_counts_are_checked_for_every_kind`, beside
//! the answer's own tests in the agents crate.
use super::super::{Hold, Settings, Slots};
use super::schema;
use super::world::{kill, open, raw, readers, sessions, settings, World, PYTHON};
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
        .execute_batch(&format!(
            "PRAGMA journal_mode=WAL; {}",
            super::world::ENDLESS
        ))
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

/// The probe of the third review (`evidence/36/verifier-report-3.md`,
/// finding 1). The read that fails and the read that names the row are
/// two processes. A writer deletes the poison row between them, so the
/// row named is the healthy one after it, and that one was skipped and
/// logged as too large.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn deleting_a_poison_row_between_reads_must_not_quarantine_its_healthy_neighbor() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    for id in ["good-before", "poison", "good-after"] {
        world.add_session(&db, id, 1);
    }
    db.execute_batch(
        "UPDATE sessions SET last_activity_at=replace(hex(zeroblob(33554432)),'0','x') WHERE id='poison';
         PRAGMA wal_checkpoint(TRUNCATE)",
    )
    .unwrap();
    // The far `python3` is a parent that runs the real reader and, once
    // that one has ended itself over the poison row, deletes the row, as
    // a far writer would, before the caller can ask anything more.
    let at = |name: &str| world.lab.at(name).display().to_string();
    let parent = [
        "import subprocess, sys, sqlite3".to_owned(),
        format!("args = open({:?}).read().split('\\n')[:-1]", at("race-arguments")),
        format!("reader = subprocess.Popen([{PYTHON:?}] + args, stdin=sys.stdin)"),
        "reader.wait()".to_owned(),
        "if reader.returncode == 5:".to_owned(),
        format!("    db = sqlite3.connect({:?})", world.database().display().to_string()),
        "    db.execute(\"DELETE FROM sessions WHERE id='poison'\")".to_owned(),
        "    db.commit()".to_owned(),
        "    db.close()".to_owned(),
        format!("    open({:?}, 'a').write('deleted the poison row after the reader ended with status 5\\n')", at("race-trace")),
        "sys.exit(reader.returncode)".to_owned(),
    ];
    std::fs::write(at("race-parent.py"), parent.join("\n") + "\n").unwrap();
    world.stand_in(&format!(
        "printf '%s\\n' \"$@\" > '{}'\nexec {PYTHON} -I '{}'",
        at("race-arguments"),
        at("race-parent.py")
    ));
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let got = reader.prints(&link, &slot, &world.home, 1, 1, None);
    let trace = std::fs::read_to_string(at("race-trace")).unwrap_or_default();
    println!("  writer: {}", trace.trim());
    println!("  page after the deletion => {got:?}");
    world.no_stand_in();
    assert_eq!(
        trace.lines().count(),
        1,
        "the row was deleted once, between the two reads"
    );
    // The row named was read by itself before anything was said of it,
    // and it reads: it is a fingerprint like any other, not a skip.
    let page = got.unwrap();
    assert!(
        !page.skipped.contains(&3),
        "deleting the poison between the failed read and rowids lookup quarantined healthy rowid 3 and advanced beyond it"
    );
    assert_eq!(page.skipped, Vec::<i64>::new());
    let ids: Vec<&str> = page.prints.iter().map(|print| print.id.as_str()).collect();
    assert_eq!(
        (ids.as_slice(), page.seen, page.last),
        (&["good-after"][..], 1, 3)
    );
    assert!(world
        .logged()
        .iter()
        .all(|line| !line.contains("quarantines")));
    // Nothing went round: three reads for the page, which are the one
    // that failed, the rowids, and the row by itself.
    let log = world.logged();
    let reads: Vec<&String> = log.iter().filter(|line| !line.contains("probe")).collect();
    assert_eq!(reads.len(), 3, "{reads:?}");
    assert!(
        reads[0].starts_with("history prints ok exit=5") && reads[0].ends_with("too large"),
        "{reads:?}"
    );
    assert!(
        reads[1].starts_with("history rowids ok exit=0"),
        "{reads:?}"
    );
    assert!(
        reads[2].starts_with("history prints ok exit=0"),
        "{reads:?}"
    );
    // The sweep ends after it.
    let after = reader
        .prints(&link, &slot, &world.home, 3, 100, None)
        .unwrap();
    assert_eq!(
        (after.prints.len(), after.skipped.len(), after.ended),
        (0, 0, true)
    );
    link.close();
}

/// The probes of the same review's finding 2. SQLite reads a declared
/// column named `rowid` in place of a row's own number, so a database
/// with such a column chose the "rowid" that was skipped and logged, and
/// with a long text in it, made the read of rowids alone load that text.
/// Hermes's schema declares no such column, and a table that does is not
/// read at all.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_shadow_rowid_column_must_not_forge_a_quarantined_rowid() {
    use super::super::Failure;
    for (column, kind, value) in [
        (
            "rowid",
            "INTEGER",
            "CASE id WHEN 'poison' THEN 9001 ELSE 2 END",
        ),
        ("_ROWID_", "INTEGER", "9001"),
        ("Oid", "INTEGER", "9001"),
        // A text in its place, 64 MiB of it.
        ("rowid", "TEXT", "replace(hex(zeroblob(33554432)),'0','x')"),
    ] {
        let Some(world) = World::start() else { return };
        let db = world.db();
        world.add_session(&db, "poison", 1);
        world.add_session(&db, "good-after", 1);
        db.execute_batch(&format!(
            "ALTER TABLE sessions ADD COLUMN {column} {kind}; UPDATE sessions SET {column}={value};
             UPDATE sessions SET last_activity_at=replace(hex(zeroblob(33554432)),'0','x') WHERE id='poison';
             PRAGMA wal_checkpoint(TRUNCATE)"
        ))
        .unwrap();
        world.measure_memory();
        let link = world.link();
        let ask = Request::Rowids {
            after: 0,
            limit: 10,
        };
        let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
        let said = String::from_utf8_lossy(&ran.stdout).into_owned();
        println!(
            "  a column {column} {kind}: rowids => {:?}, {:?}",
            ran.end,
            said.lines().last()
        );
        assert_eq!(ran.end, End::Ended(0));
        assert_eq!(
            far::parse(&ask, &ran.stdout),
            Ok(Reply::Failed(far::Far::Rowid))
        );
        let (reader, slots) = (world.reader(settings()), Slots::new());
        let slot = slots.of(&world.machine());
        for after in [0, 2] {
            let got = reader.prints(&link, &slot, &world.home, after, 1, None);
            println!("  the caller from {after} => {got:?}");
            assert!(
                !matches!(got, Ok(ref page) if page.skipped.contains(&9001)),
                "a database column named rowid forged quarantined rowid 9001; its real rowid is 1, and cursor 2 is already past the table"
            );
            assert_eq!(got, Err(Failure::Far(far::Far::Rowid)));
        }
        // Nothing of such a table is read by id either, and no reader
        // came near the long values: the names of a table's columns are
        // all that was looked at.
        let read = reader.sessions(&link, &slot, &world.home, &["good-after".to_owned()]);
        assert_eq!(read.stopped, Some(Failure::Far(far::Far::Rowid)));
        assert!(read.read.is_empty());
        let log = world.logged();
        assert!(
            log.iter()
                .all(|line| !line.contains("quarantines") && !line.contains("9001")),
            "{log:?}"
        );
        assert!(log
            .iter()
            .any(|line| line.ends_with("state.db declares a column that hides its row numbers")));
        let peaks = world.peaks();
        println!(
            "  whole-life peaks of its {} readers: at most {} KiB",
            peaks.len(),
            peaks.iter().max().unwrap()
        );
        assert!(peaks.iter().all(|peak| *peak < MOST_MEMORY), "{peaks:?}");
        link.close();
    }
    // The usage table is held to the same.
    let Some(world) = World::start() else { return };
    let db = world.db();
    world.add_session(&db, "s1", 1);
    db.execute_batch("ALTER TABLE session_model_usage ADD COLUMN oid INTEGER")
        .unwrap();
    let link = world.link();
    let ask = sessions(&["s1"]);
    let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
    assert_eq!(
        far::parse(&ask, &ran.stdout),
        Ok(Reply::Failed(far::Far::Rowid))
    );
    // And Hermes's own schema, which declares none, is read.
    db.execute_batch("ALTER TABLE session_model_usage DROP COLUMN oid")
        .unwrap();
    let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
    assert!(
        matches!(far::parse(&ask, &ran.stdout), Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1)
    );
    link.close();
}

/// The probe of the fourth review (`evidence/36/verifier-report-4.md`).
/// The names of a table's columns were read from the list that leaves
/// out generated columns, and SQLite reads a generated column named
/// `rowid` in place of a row's own number as it does a plain one. So such
/// a table was read: it forged the rowid that was skipped, made a table
/// of healthy sessions read as empty, and made the read of rowids alone
/// hold a long value.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn r4_a_generated_column_named_rowid_must_be_refused_like_a_plain_one() {
    use super::super::Failure;
    let long = "replace(hex(zeroblob(33554432)),'0','x')";
    let cases = [
        // The third review's own probe, with the column generated.
        (
            "a number of its choosing",
            "rowid",
            "CASE id WHEN 'poison' THEN 9001 ELSE 2 END",
        ),
        // Zero for every row: no row is past zero, so a sweep read the
        // table as empty and ended, with nothing logged.
        ("zero for every row", "rowid", "0"),
        // 64 MiB of text, which the read of rowids alone held.
        ("64 MiB of text", "rowid", long),
        ("another spelling", "RowID", "7"),
        ("the second name", "_rowid_", "7"),
        ("the third name", "OID", "7"),
    ];
    for (what, column, value) in cases {
        let Some(world) = World::start() else { return };
        let db = world.db();
        for id in ["poison", "good-after", "good-last"] {
            world.add_session(&db, id, 1);
        }
        db.execute_batch(&format!(
            "ALTER TABLE sessions ADD COLUMN {column} GENERATED ALWAYS AS ({value}) VIRTUAL;
             UPDATE sessions SET last_activity_at={long} WHERE id='poison';
             PRAGMA wal_checkpoint(TRUNCATE)"
        ))
        .unwrap();
        // The plain list of columns does not hold it. The whole list does.
        let listed = |list: &str| -> i64 {
            let sql = format!(
                "SELECT COUNT(*) FROM {list}('sessions') WHERE lower(name) IN ('rowid','_rowid_','oid')"
            );
            db.query_row(&sql, [], |row| row.get(0)).unwrap()
        };
        assert_eq!(
            (listed("pragma_table_info"), listed("pragma_table_xinfo")),
            (0, 1),
            "{what}"
        );
        world.measure_memory();
        let link = world.link();
        let ask = Request::Rowids {
            after: 0,
            limit: 10,
        };
        let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
        let said = String::from_utf8_lossy(&ran.stdout).into_owned();
        println!(
            "  a generated {column}, {what}: rowids => {:?}, {:?}",
            ran.end,
            said.lines().last()
        );
        assert_eq!(ran.end, End::Ended(0), "{what}");
        assert_eq!(
            far::parse(&ask, &ran.stdout),
            Ok(Reply::Failed(far::Far::Rowid)),
            "{what}"
        );
        let (reader, slots) = (world.reader(settings()), Slots::new());
        let slot = slots.of(&world.machine());
        for after in [0, 2] {
            let got = reader.prints(&link, &slot, &world.home, after, 100, None);
            println!("  the caller from {after} => {got:?}");
            // Not a forged skip, not an empty table, not a reader ended
            // over the value: the table is not read, and says why.
            assert_eq!(got, Err(Failure::Far(far::Far::Rowid)), "{what}");
        }
        let read = reader.sessions(&link, &slot, &world.home, &["good-after".to_owned()]);
        assert_eq!(read.stopped, Some(Failure::Far(far::Far::Rowid)), "{what}");
        let log = world.logged();
        assert!(
            log.iter()
                .all(|line| !line.contains("quarantines") && !line.contains("9001")),
            "{log:?}"
        );
        let peaks = world.peaks();
        println!(
            "  whole-life peaks of its {} readers: at most {} KiB",
            peaks.len(),
            peaks.iter().max().unwrap()
        );
        assert!(
            peaks.iter().all(|peak| *peak < MOST_MEMORY),
            "{what}: {peaks:?}"
        );
        link.close();
    }
    // A stored one, which only a table made with it can have, and one in
    // the usage table.
    let Some(world) = World::start() else { return };
    let db = world.db();
    world.add_session(&db, "s1", 1);
    db.execute_batch(
        "ALTER TABLE session_model_usage ADD COLUMN rowid GENERATED ALWAYS AS (3) VIRTUAL;",
    )
    .unwrap();
    let link = world.link();
    let ask = sessions(&["s1"]);
    let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
    assert_eq!(
        far::parse(&ask, &ran.stdout),
        Ok(Reply::Failed(far::Far::Rowid))
    );
    db.execute_batch(
        "ALTER TABLE session_model_usage DROP COLUMN rowid;
         ALTER TABLE sessions RENAME TO kept;
         CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at REAL NOT NULL,
            rowid INTEGER GENERATED ALWAYS AS (9001) STORED);
         INSERT INTO sessions (id, source, started_at) SELECT id, source, started_at FROM kept;",
    )
    .unwrap();
    let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
    assert_eq!(
        far::parse(&ask, &ran.stdout),
        Ok(Reply::Failed(far::Far::Rowid))
    );
    // A generated column of another name is a column like any other, and
    // the table is read.
    db.execute_batch(
        "DROP TABLE sessions; ALTER TABLE kept RENAME TO sessions;
         ALTER TABLE sessions ADD COLUMN shown GENERATED ALWAYS AS (id || '!') VIRTUAL;",
    )
    .unwrap();
    let ran = raw(&link, &world.home, Duration::from_secs(5), &ask);
    assert!(
        matches!(far::parse(&ask, &ran.stdout), Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1)
    );
    link.close();
}
