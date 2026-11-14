//! The reader closes the database before it answers, and its own deadline
//! ends it whatever it is doing.
use super::super::{Failure, Slots};
use super::world::{machine, most, open, raw, readers, sessions, settings, Watch, World, PYTHON};
use crate::ssh_link::unix::tests::lab::until;
use crate::ssh_link::End;
use agent_hud_agents::hermes::far_history::{self as far, Reply, Request};
use rusqlite::Connection;
use std::{
    ffi::CString,
    fs::{self, File},
    io::Read,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    time::{Duration, Instant},
};

/// A database whose every session read is about a quarter of a megabyte:
/// more than a pipe holds, so a reader whose output nobody takes blocks.
fn big(world: &World) -> Vec<String> {
    let db = world.db();
    let ids: Vec<String> = (0..30).map(|number| format!("s-{number:02}")).collect();
    for id in &ids {
        world.add_session(&db, id, 60);
    }
    ids
}

/// Sends the far reader's output into a pipe this test holds and does not
/// read: the far side of a link that takes nothing.
fn hold_back_the_output(world: &World) -> File {
    let pipe = world.lab.at("held-back");
    let name = CString::new(pipe.display().to_string()).unwrap();
    // SAFETY: makes a named pipe in the lab's own folder.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let held = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&pipe)
        .unwrap();
    world.stand_in(&format!("exec {PYTHON} \"$@\" > '{}'", pipe.display()));
    held
}

/// How many bytes wait in the pipe.
fn waiting(pipe: &File) -> usize {
    let mut count: libc::c_int = 0;
    // SAFETY: asks how much can be read from a descriptor this test holds.
    unsafe { libc::ioctl(pipe.as_raw_fd(), libc::FIONREAD, &mut count) };
    count as usize
}

/// A checkpoint that resets the write-ahead log, as a far writer makes
/// one: whether it was held up, and the log's length afterwards.
fn checkpoint(world: &World, db: &Connection) -> (i64, u64) {
    let busy = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    let log = world.database().with_extension("db-wal");
    (busy, fs::metadata(log).map(|file| file.len()).unwrap_or(0))
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_database_is_closed_before_the_first_byte_of_output() {
    let Some(world) = World::start() else { return };
    let ids = big(&world);
    let db = world.db();
    db.busy_timeout(Duration::ZERO).unwrap();
    let write = |title: &str| {
        db.execute("UPDATE sessions SET title=?1 WHERE id='s-00'", [title])
            .unwrap();
    };

    // What the check can see: a reader that still has its read open holds
    // the checkpoint up, and the log is not reset.
    let other = world.db();
    let mut statement = other.prepare("SELECT id FROM sessions").unwrap();
    let mut rows = statement.query([]).unwrap();
    rows.next().unwrap();
    write("changed while a reader is open");
    let (busy, log) = checkpoint(&world, &db);
    println!("  with a reader open: busy={busy}, the log is {log} bytes");
    assert!(busy == 1 && log > 0);
    drop(rows);
    drop(statement);
    assert_eq!(checkpoint(&world, &db), (0, 0));
    write("as it was read");

    // The far reader, with its output held back.
    let mut held = hold_back_the_output(&world);
    let link = world.link();
    let asked: Vec<&str> = ids.iter().map(String::as_str).collect();
    let request = sessions(&asked);
    let ran = std::thread::scope(|scope| {
        let run = scope.spawn(|| raw(&link, &world.home, Duration::from_secs(12), &request));
        // Its rows have begun to arrive and cannot all fit: it is blocked
        // in its write, alive, with everything it read in memory.
        assert!(until(Duration::from_secs(20), || waiting(&held) > 4096));
        std::thread::sleep(Duration::from_millis(300));
        let pids = readers(&world.mark());
        let pending = waiting(&held);
        println!("  the reader {pids:?} is blocked with {pending} bytes waiting");
        assert_eq!(pids.len(), 1);
        assert!(!run.is_finished());
        // It does not have the database open any more.
        assert_eq!(
            open(&pids, &world.database().display().to_string()),
            Vec::<u32>::new()
        );
        // So a writer on the far side can checkpoint and reset the log.
        write("changed while the reader's output is held back");
        let (busy, log) = checkpoint(&world, &db);
        println!("  with the reader's output held back: busy={busy}, the log is {log} bytes");
        assert_eq!((busy, log), (0, 0));

        // Now the output is taken, and it is whole: what the reader had
        // read before the write above.
        let mut answer = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        while !run.is_finished() || waiting(&held) > 0 {
            match held.read(&mut chunk) {
                Ok(count) if count > 0 => answer.extend_from_slice(&chunk[..count]),
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        let read = far::parse(&request, &answer);
        let Ok(Reply::Sessions(batch)) = read else {
            panic!("{read:?}");
        };
        println!(
            "  {} bytes then came, {} sessions whole",
            answer.len(),
            batch.sessions.len()
        );
        assert!(answer.len() > 200_000);
        assert_eq!(batch.sessions.len(), 30);
        assert!(batch
            .sessions
            .iter()
            .all(|session| session.usage.len() == 60));
        assert_eq!(
            batch.sessions[0].cells[1],
            far::Cell::Text("as it was read".into())
        );
        run.join().unwrap()
    });
    assert_eq!(ran.end, End::Ended(0), "{}", ran.stderr);
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_deadline_ends_a_busy_query() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let watch = Watch::start(&world);
    let began = Instant::now();
    let read = reader.read(&link, &slot, &world.home, &sessions(&["endless"]));
    let took = began.elapsed();
    // The watchdog's own status: not an answer, and not the system timer.
    assert_eq!(read, Err(Failure::Exit(3)));
    assert!(world.logged().pop().unwrap().contains("exit=3"));
    // The far process is gone.
    assert_eq!(readers(&world.mark()), Vec::<u32>::new());
    let samples = watch.finish();
    assert_eq!(most(&samples), (1, 1));
    let with: Vec<Instant> = samples
        .iter()
        .filter(|look| !look.open.is_empty())
        .map(|look| look.at)
        .collect();
    let open = *with.last().unwrap() - with[0];
    println!(
        "  the read took {took:?}; the reader had the database open for {open:?} of a 2 s deadline"
    );
    assert!(
        open > Duration::from_millis(1_300) && open < Duration::from_millis(2_900),
        "{open:?}"
    );
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(8),
        "{took:?}"
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_deadline_ends_a_blocked_write() {
    let Some(world) = World::start() else { return };
    let ids = big(&world);
    // Nothing ever takes the reader's output.
    let held = hold_back_the_output(&world);
    let link = world.link();
    let asked: Vec<&str> = ids.iter().map(String::as_str).collect();
    let watch = Watch::start(&world);
    let began = Instant::now();
    let ran = raw(
        &link,
        &world.home,
        Duration::from_secs(2),
        &sessions(&asked),
    );
    let took = began.elapsed();
    assert_eq!(ran.end, End::Ended(3), "{}", ran.stderr);
    assert_eq!(readers(&world.mark()), Vec::<u32>::new());
    let samples = watch.finish();
    // It was blocked in the write, not in the read: part of its answer
    // waits in the pipe, more than its first line and less than the whole.
    let pending = waiting(&held);
    let seen: Vec<Instant> = samples
        .iter()
        .filter(|look| !look.readers.is_empty())
        .map(|look| look.at)
        .collect();
    let lived = *seen.last().unwrap() - seen[0];
    println!("  ended with status 3 after {took:?}; alive for {lived:?}; {pending} bytes of its answer wait unread");
    assert!(pending > 4096 && pending < 200_000, "{pending}");
    assert!(
        lived > Duration::from_millis(1_800) && lived < Duration::from_secs(5),
        "{lived:?}"
    );
    assert_eq!(most(&samples).0, 1);
    link.close();
}
