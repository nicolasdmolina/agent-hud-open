//! What ticket 54 changed where a write or a read of a mirror fails:
//!
//! - A mirror core could not read is read again as soon as its writer
//!   says it wrote it, not 30 s later.
//! - No file without its tables is ever at a mirror's place: a first
//!   write that is given up leaves none.
//! - A write that fails the same way at every poll is logged once.
//! - A new label is never the write a full queue drops.
use super::*;

/// What is in the folder of the mirror of `profile`.
fn beside(bench: &Bench, profile: &Profile) -> Vec<String> {
    let folder = bench.file(profile);
    let entries = std::fs::read_dir(folder.parent().unwrap());
    let mut names: Vec<String> = (entries.into_iter().flatten().flatten())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A mirror that was made and is open: its file, and what SQLite keeps
/// beside one in write-ahead mode. Nothing of its making is left.
const MADE: [&str; 3] = ["state.db", "state.db-shm", "state.db-wal"];

/// How long until core reads again with nothing to wake it.
fn next_pass(bench: &Bench) -> Duration {
    let due = bench.core.next_pass_due().expect("a walk is always due");
    due.saturating_duration_since(Instant::now())
}

/// A database with no table in it at `path`, as the writer of ticket 34
/// left one whose first write was given up.
fn without_tables(path: &Path) {
    for folder in [3, 2, 1].map(|up| path.ancestors().nth(up).unwrap()) {
        crate::os::private_directory(folder).unwrap();
    }
    let db = rusqlite::Connection::open(path).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    assert!(std::fs::metadata(path).unwrap().len() > 0);
}

#[test]
fn a_mirror_that_could_not_be_read_is_listed_once_its_writer_tells() {
    let mut bench = bench("Mac mini");
    let path = bench.file(&Profile::Default);
    without_tables(&path);
    // The file watcher reports the new file; core reads it and cannot.
    bench.core.file_changes(Some(HashSet::from([path.clone()])));
    bench.core.request("snapshot", json!({})).unwrap();
    let paused = next_pass(&bench);
    assert!(
        (Duration::from_secs(20)..=Duration::from_secs(30)).contains(&paused),
        "its next read is due in {paused:?}"
    );
    assert!(!bench.core.indexing(), "a pausing file is not waited for");
    // Nor does another file event end the pause.
    bench.core.file_changes(Some(HashSet::from([path.clone()])));
    bench.core.request("snapshot", json!({})).unwrap();
    assert!(next_pass(&bench) >= Duration::from_secs(20));

    // The next poll: the writer makes the file a mirror, and says so.
    bench.lists(&Profile::Default, &[("s1", "First")], 1_000, 1);
    let committed = *bench.told.at.lock().unwrap().last().unwrap();
    assert_eq!(bench.turn(), 1);
    let ids = bench.ids();
    let took = committed.elapsed();
    assert_eq!(ids, ["default:s1"]);
    assert!(took <= SOON, "{took:?}");
    eprintln!("after a failed read: listed {took:?} after its commit");
}

#[test]
fn a_first_write_that_is_given_up_leaves_no_file() {
    let mut bench = bench("Mac mini");
    let path = bench.file(&Profile::Default);
    // The route changes between the writer's look and its commit.
    bench.number.raise_at.store(2, Ordering::Release);
    bench.lists(&Profile::Default, &[("s1", "First")], 1_000, 1);
    assert_eq!(bench.number.now.load(Ordering::Acquire), 2);
    assert_eq!(beside(&bench, &Profile::Default), [""; 0]);
    assert_eq!(bench.tells(), 0);
    // A walk of every root finds nothing it cannot read: nothing is due
    // before the next walk.
    bench.core.request("refresh", json!({})).unwrap();
    bench.silent_pass();
    assert!(next_pass(&bench) > Duration::from_secs(200));

    // Under the number as it is now, the mirror is made, and whole.
    bench.lists(&Profile::Default, &[("s1", "First")], 2_000, 2);
    assert_eq!(beside(&bench, &Profile::Default), MADE);
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = rusqlite::Connection::open_with_flags(&path, flags).unwrap();
    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(db);
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.ids(), ["default:s1"]);
    // And it goes on taking writes, each told.
    bench.lists(
        &Profile::Default,
        &[("s1", "First"), ("s2", "Second")],
        3_000,
        2,
    );
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.ids(), ["default:s1", "default:s2"]);
}

/// A writer that was cut off while it made a mirror left its file under
/// the name it is made by. The next one starts over.
#[test]
fn a_mirror_left_half_made_is_made_anew() {
    let mut bench = bench("Mac mini");
    let path = bench.file(&Profile::Default);
    let making = path.with_file_name("state.db.new");
    without_tables(&making);
    // What it had got to write is of no use: here, another machine's.
    let left = rusqlite::Connection::open(&making).unwrap();
    left.execute_batch(
        "CREATE TABLE hud_mirror (schema INTEGER, machine TEXT, label TEXT, profile TEXT);
         INSERT INTO hud_mirror VALUES (1, 'ffffffffffffffff', 'Another', 'default');",
    )
    .unwrap();
    drop(left);
    // Core's walk does not take it for a mirror.
    bench.core.request("refresh", json!({})).unwrap();
    assert!(next_pass(&bench) > Duration::from_secs(200));

    bench.lists(&Profile::Default, &[("s1", "First")], 1_000, 1);
    assert_eq!(beside(&bench, &Profile::Default), MADE);
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.ids(), ["default:s1"]);
}

#[test]
fn a_write_that_fails_at_every_poll_is_logged_once_and_leaves_no_file() {
    // A machine with no label, which no mirror can be claimed for. The
    // list's reader gives every followed machine one; the writer still
    // holds its own.
    let mut bench = bench("");
    for poll in 0..5 {
        bench.lists(&Profile::Default, &[("s1", "First")], 1_000 + poll, 1);
    }
    let failed = "mirror failed profile=default why=refused";
    assert_eq!(bench.log().matches(failed).count(), 1, "{}", bench.log());
    assert_eq!(beside(&bench, &Profile::Default), [""; 0]);
    assert_eq!(bench.tells(), 0);
    bench.core.request("refresh", json!({})).unwrap();
    assert!(next_pass(&bench) > Duration::from_secs(200));

    // Named, it gets its rows; and a failure after a good write is news.
    bench.writer.relabel("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "First")], 2_000, 1);
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.ids(), ["default:s1"]);
    std::fs::remove_dir_all(bench.file(&Profile::Default).parent().unwrap()).unwrap();
    bench.writer.relabel("");
    bench.lists(&Profile::Default, &[("s1", "First")], 3_000, 1);
    bench.lists(&Profile::Default, &[("s1", "First")], 4_000, 1);
    assert_eq!(bench.log().matches(failed).count(), 2, "{}", bench.log());
}

/// The writer's commit step waits, as on a disk that stalls.
struct Stalled {
    shut: Mutex<bool>,
    moved: Condvar,
}

impl Routes for Stalled {
    fn under(&self, _number: u64, commit: &mut dyn FnMut()) -> bool {
        let mut shut = self.shut.lock().unwrap();
        while *shut {
            shut = self.moved.wait(shut).unwrap();
        }
        drop(shut);
        commit();
        true
    }
}

#[test]
fn a_new_label_is_never_the_write_a_full_queue_drops() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().canonicalize().unwrap().join("data");
    crate::os::private_directory(&data).unwrap();
    let disk = Arc::new(Stalled {
        shut: Mutex::new(false),
        moved: Condvar::new(),
    });
    let untold: Changed = Arc::new(|_| {});
    let log = Log::new(&data).of(Some("mirror"));
    let writer = Writer::start(&data, (&key(), "Old"), disk.clone(), untold, log).unwrap();
    writer.listed(listing(&Profile::Default, &[("s1", "First")], 1_000), 1);
    writer.listed(listing(&reports(), &[("r1", "First")], 1_000), 1);
    writer.settle();

    // The disk stalls: the thread takes one write and waits in it.
    *disk.shut.lock().unwrap() = true;
    writer.listed(listing(&Profile::Default, &[("s2", "Second")], 2_000), 1);
    let limit = Instant::now() + Duration::from_secs(30);
    while !writer.state().busy {
        assert!(Instant::now() < limit, "the write was never taken");
        std::thread::sleep(Duration::from_millis(2));
    }
    // The entry is renamed, twice, while the polls go on listing: more
    // of them than the queue holds.
    writer.relabel("Newer");
    writer.relabel("New");
    for poll in 0..2 * MOST as i64 {
        let read_ms = 3_000 + poll;
        writer.listed(listing(&Profile::Default, &[("s3", "Third")], read_ms), 1);
    }
    // The queue holds its fill of listings, and the one label write.
    let waiting = writer.state().queue.len();
    assert_eq!(waiting, MOST + 1);
    *disk.shut.lock().unwrap() = false;
    disk.moved.notify_all();
    writer.settle();

    let label = |profile: &Profile| {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let db = rusqlite::Connection::open_with_flags(writer.file(profile), flags).unwrap();
        let said = db.query_row("SELECT label FROM hud_mirror", [], |row| row.get(0));
        said.unwrap_or_else(|_| String::new())
    };
    assert_eq!(label(&Profile::Default), "New");
    assert_eq!(label(&reports()), "New");
    writer.stop();
}

/// Counts that arrive while the disk stalls do not pile up (ticket 60):
/// one write waits for a profile, whatever the rounds, and it says the
/// newest of every session any of them asked.
#[test]
fn counts_that_wait_for_the_disk_are_one_write_for_a_profile() {
    use agent_hud_agents::hermes::gateway::Sample;
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().canonicalize().unwrap().join("data");
    crate::os::private_directory(&data).unwrap();
    let disk = Arc::new(Stalled {
        shut: Mutex::new(false),
        moved: Condvar::new(),
    });
    let untold: Changed = Arc::new(|_| {});
    let log = Log::new(&data).of(Some("mirror"));
    let space = Duration::from_millis(1);
    let named = (&key(), "Mini");
    let writer = Writer::spaced(&data, named, disk.clone(), untold, log, space).unwrap();
    let rows = [("s1", "First"), ("s2", "Second")];
    writer.listed(listing(&Profile::Default, &rows, 1_000), 1);
    writer.listed(listing(&reports(), &[("r1", "First")], 1_000), 1);
    writer.settle();

    // The disk stalls: the thread takes one write and waits in it.
    *disk.shut.lock().unwrap() = true;
    writer.listed(listing(&Profile::Default, &[("s3", "Third")], 2_000), 1);
    let limit = Instant::now() + Duration::from_secs(30);
    while !writer.state().busy {
        assert!(Instant::now() < limit, "the write was never taken");
        std::thread::sleep(Duration::from_millis(2));
    }
    let said = |profile: &Profile, sessions: &[(&str, u64)]| {
        let sample = |total: u64| Sample {
            input: total,
            ..Sample::default()
        };
        let samples = sessions
            .iter()
            .map(|(id, total)| ((*id).to_owned(), sample(*total)));
        BTreeMap::from([(profile.clone(), samples.collect::<Vec<_>>())])
    };
    // A hundred rounds: the first asks both sessions, the rest only one.
    writer.counted(said(&Profile::Default, &[("s1", 10), ("s2", 77)]), 3_000, 1);
    for round in 1..=100 {
        writer.counted(
            said(&Profile::Default, &[("s1", 10 + round)]),
            3_000 + round as i64,
            1,
        );
        writer.counted(said(&reports(), &[("r1", round)]), 3_000 + round as i64, 1);
    }
    // One write of counts waits for each profile.
    assert_eq!(writer.state().queue.len(), 2);
    *disk.shut.lock().unwrap() = false;
    disk.moved.notify_all();
    writer.settle();
    std::thread::sleep(Duration::from_millis(50));
    writer.settle();

    let kept = |profile: &Profile| {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let db = rusqlite::Connection::open_with_flags(writer.file(profile), flags).unwrap();
        let sql = "SELECT session_id, input_tokens FROM hud_live ORDER BY session_id";
        let mut statement = db.prepare(sql).unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)));
        rows.unwrap()
            .map(Result::unwrap)
            .collect::<Vec<(String, i64)>>()
    };
    // The newest of the session every round asked, and what the first
    // round said of the one no later round asked.
    assert_eq!(
        kept(&Profile::Default),
        [("s1".to_owned(), 110), ("s2".to_owned(), 77)]
    );
    assert_eq!(kept(&reports()), [("r1".to_owned(), 100)]);
    writer.stop();
}
