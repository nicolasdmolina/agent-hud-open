//! A database nobody has open, as a profile whose gateway is stopped
//! leaves it: in write-ahead mode, with no log and no shared memory file.
//! The reader opens a database one way only, through a connection that
//! cannot write, and never as a database that cannot change. Apple's
//! SQLite, which the far `python3` of this lab has, will not open such a
//! database that way, so its history is not read until Hermes runs there
//! again. This is a limit of the design, pinned here.
use super::super::{Failure, Slots};
use super::world::{machine, settings, World};
use agent_hud_agents::hermes::far_history::Far;
use std::fs;

fn owned(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// What only Apple's SQLite does, which is what the far Python has here.
fn apples() -> bool {
    cfg!(target_os = "macos")
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_database_nobody_has_open_is_named_as_not_opened_and_nothing_is_made_beside_it() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 2);
    world.stop_hermes();
    assert_eq!(world.files(), ["state.db"]);
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let read = reader.sessions(&link, &slot, &world.home, &owned(&["s1"]));
    println!(
        "  read {} session(s), stopped: {:?}; the home now holds {:?}",
        read.read.len(),
        read.stopped,
        world.files()
    );
    match read.stopped {
        Some(failure) => {
            assert_eq!(failure, Failure::Far(Far::Open));
            assert_eq!(failure.to_string(), "state.db could not be opened");
            assert!(read.read.is_empty());
            assert_eq!(read.unread, ["s1"]);
            // Nothing was made beside the database.
            assert_eq!(world.files(), ["state.db"]);
        }
        // A SQLite that opens it reads it, and leaves a log and a shared
        // memory file behind.
        None => assert_eq!(read.read.len(), 1),
    }
    if apples() {
        assert_eq!(read.stopped, Some(Failure::Far(Far::Open)));
        let page = reader.prints(&link, &slot, &world.home, 0, 10, None);
        assert_eq!(page, Err(Failure::Far(Far::Open)));
    }
    // Once Hermes has it open again, it is read.
    let hermes = world.db();
    world.add_session(&hermes, "s2", 1);
    let read = reader.sessions(&link, &slot, &world.home, &owned(&["s1", "s2"]));
    assert_eq!((read.read.len(), read.stopped), (2, None));
    link.close();
}

/// A writer that did not close cleanly leaves its log, with rows in it
/// that the database file does not have. The reader opens the database
/// with its log, so those rows come too.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_log_left_behind_is_read_with_the_rows_that_are_only_in_it() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    world.add_session(&db, "s1", 1);
    // The file and the log are kept as they are while a row is only in
    // the log, and put back once Hermes has stopped.
    let busy: i64 = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0);
    db.execute_batch("PRAGMA wal_autocheckpoint=0").unwrap();
    world.add_session(&db, "only-in-the-log", 1);
    let log = world.database().with_extension("db-wal");
    let (file_aside, log_aside) = (world.lab.at("file-aside"), world.lab.at("log-aside"));
    fs::copy(world.database(), &file_aside).unwrap();
    fs::copy(&log, &log_aside).unwrap();
    drop(db);
    world.stop_hermes();
    assert_eq!(world.files(), ["state.db"]);
    fs::copy(&file_aside, world.database()).unwrap();
    fs::copy(&log_aside, &log).unwrap();
    assert!(fs::metadata(&log).unwrap().len() > 0);

    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let asked = owned(&["s1", "only-in-the-log"]);
    let read = reader.sessions(&link, &slot, &world.home, &asked);
    // The shared memory file is made by the open: no intentional write,
    // and not "nothing written".
    println!(
        "  read {} session(s), stopped {:?}; the home now holds {:?}",
        read.read.len(),
        read.stopped,
        world.files()
    );
    assert_eq!(read.stopped, None);
    let came: Vec<&str> = read
        .read
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(came, ["s1", "only-in-the-log"]);
    // The log is as it was left: a reader does not fold it in.
    assert_eq!(fs::read(&log).unwrap(), fs::read(&log_aside).unwrap());
    assert_eq!(
        fs::read(world.database()).unwrap(),
        fs::read(&file_aside).unwrap()
    );
    link.close();
}
