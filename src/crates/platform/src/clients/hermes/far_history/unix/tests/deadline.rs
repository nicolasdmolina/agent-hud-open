//! The reader closes the database before it answers, its own deadline
//! ends it whatever it is doing, and it never ends through Python's own
//! shutdown.
use super::super::{Failure, Settings, Slots};
use super::world::{most, open, raw, readers, sessions, settings, Watch, World, PYTHON};
use crate::ssh_link::unix::tests::lab::until;
use crate::ssh_link::End;
use agent_hud_agents::hermes::far_history::{self as far, Reply, Request};
use rusqlite::Connection;
use std::{
    ffi::CString,
    fs::{self, File},
    io::Read,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    thread,
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
    let slot = slots.of(&world.machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    // The far `python3` becomes a parent that starts the reader, passes on
    // what it says, and notes by the wall clock when its `started` went by
    // and when its process ended: how long it held the database, taken on
    // the far side. It takes the reader's arguments from a file, so it is
    // no reader to anyone (ticket 49).
    let at = |name: &str| world.lab.at(name).display().to_string();
    fs::write(
        at("held.py"),
        format!(
            "import os, subprocess, sys, time\n\
             args = open({args:?}).read().split('\\n')[:-1]\n\
             child = subprocess.Popen([{PYTHON:?}] + args, stdin=sys.stdin, stdout=subprocess.PIPE)\n\
             os.close(0)\n\
             started = 0.0\n\
             for line in child.stdout:\n\
             \x20   if not started and line.startswith(b'[\"started\",'):\n\
             \x20       started = time.time()\n\
             \x20   os.write(1, line)\n\
             status = child.wait()\n\
             open({times:?}, 'a').write('%r %r\\n' % (started, time.time()))\n\
             sys.exit(status % 256)\n",
            args = at("held-arguments"),
            times = at("held-times"),
        ),
    )
    .unwrap();
    world.stand_in(&format!(
        "printf '%s\\n' \"$@\" > '{}'\nexec {PYTHON} -I '{}'",
        at("held-arguments"),
        at("held.py")
    ));
    let watch = Watch::start(&world);
    // The busy read has a 6 s deadline, so the looks at the far side's
    // open files, a second or more apart on a busy machine, see it with
    // the database open. With 2 s they could all miss it (ticket 49).
    let deadline = Duration::from_secs(6);
    let busy = world.reader(Settings {
        deadline,
        local_wait: Duration::from_secs(30),
        ..settings()
    });
    let margin = busy.settings().margin;
    let began = Instant::now();
    let read = busy.read(&link, &slot, &world.home, &sessions(&["endless"]));
    let took = began.elapsed();
    // The watchdog's own status: not an answer, and not the system timer.
    assert_eq!(read, Err(Failure::Exit(3)));
    assert!(world.logged().pop().unwrap().contains("exit=3"));
    // The far process is gone.
    assert_eq!(readers(&world.mark()), Vec::<u32>::new());
    let samples = watch.finish();
    // It had the database open, and nothing beside it did.
    assert_eq!(most(&samples), (1, 1));
    let times = fs::read_to_string(at("held-times")).unwrap();
    let times: Vec<f64> = times
        .split_whitespace()
        .map(|time| time.parse().unwrap())
        .collect();
    let open = Duration::from_secs_f64(times[1] - times[0]);
    println!(
        "  the read took {took:?}; the reader ran {open:?} from its `started`, of a {deadline:?} deadline"
    );
    // From its `started` to its end it ran its deadline, less a second
    // (the parent may see `started` a little late), and never past it and
    // the margin. Before, this side's looks at the open files timed it
    // (over 600 ms of a 2 s deadline seen open), and on a busy machine
    // they came a second or more apart (ticket 49). How long the reader
    // took to start is this machine's business: it has twenty seconds.
    assert!(
        times[0] > 0.0 && open > deadline - Duration::from_secs(1) && open < deadline + margin,
        "{open:?}"
    );
    assert!(
        took >= deadline && took < deadline + margin + Duration::from_secs(20),
        "{took:?}"
    );
    link.close();
}

/// The deadline is counted from the reader's `started`, and so is this
/// test: from the moment that line reaches the pipe the test holds, to
/// the moment the link returns the reader's status. How long the reader
/// took to start is outside that, so the bound is tight on any machine: a
/// 2 s deadline, and no more than the 2 s margin the caller's slot allows
/// for the status to come back. An earlier form of this test counted from
/// the launch and allowed 23 s, and a reader that lived 15 s on its 2 s
/// deadline passed it (fifth review).
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
    let done = AtomicBool::new(false);
    let (ran, started, ended) = thread::scope(|scope| {
        // `started` is the first thing in the pipe.
        let first_byte = scope.spawn(|| {
            while !done.load(Ordering::SeqCst) {
                if waiting(&held) > 0 {
                    return Some(Instant::now());
                }
                thread::sleep(Duration::from_millis(1));
            }
            None
        });
        let ran = raw(
            &link,
            &world.home,
            Duration::from_secs(2),
            &sessions(&asked),
        );
        let ended = Instant::now();
        done.store(true, Ordering::SeqCst);
        (ran, first_byte.join().unwrap(), ended)
    });
    let after = ended - started.expect("the reader wrote `started`");
    assert_eq!(ran.end, End::Ended(3), "{}", ran.stderr);
    assert_eq!(readers(&world.mark()), Vec::<u32>::new());
    let samples = watch.finish();
    // It was blocked in the write, not in the read: part of its answer
    // waits in the pipe, more than its first line and less than the whole.
    let pending = waiting(&held);
    println!(
        "  ended with status 3, {after:?} after its `started` reached the pipe; {pending} bytes of its answer wait unread"
    );
    assert!(pending > 4096 && pending < 200_000, "{pending}");
    assert!(
        after > Duration::from_millis(1_800) && after < Duration::from_secs(4),
        "{after:?}"
    );
    assert_eq!(most(&samples).0, 1);
    link.close();
}

/// Whatever a reader answers, it ends at once and not through Python's
/// own shutdown. Its watchdog thread is still running then, and a thread
/// that wakes just as the shutdown frees its state reads freed memory:
/// Python 3.9 crashed that way under heavy load (ticket 55). Here an exit
/// hook, which only that shutdown runs, would make a file.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_reader_never_ends_through_pythons_own_shutdown() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 2);
    let ran = world.lab.at("shutdown-ran");
    let hook = format!("import atexit, sys; atexit.register(open, {ran:?}, \"w\")");
    // The hook does make the file where a Python ends the usual way.
    let usual = Command::new(PYTHON).args(["-I", "-c", &hook]).status();
    assert!(usual.expect("the far Python").success() && ran.exists());
    fs::remove_file(&ran).unwrap();
    world.stand_in(&format!(
        "shift 2\nexec {PYTHON} -I -c '{hook}; exec(compile(sys.stdin.read(), \"reader\", \"exec\"))' \"$@\""
    ));
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    // One that reads nothing, each kind of read, and one that is refused.
    let probe = reader.read(&link, &slot, &world.home, &Request::Probe);
    assert_eq!(probe, Ok(Reply::Probed));
    let page = reader.prints(&link, &slot, &world.home, 0, 10, None);
    assert_eq!(page.map(|page| page.prints.len()), Ok(1));
    let read = reader.sessions(&link, &slot, &world.home, &["s1".to_owned()]);
    assert_eq!((read.read.len(), read.stopped), (1, None));
    assert_eq!(reader.held(&link, &slot, &world.home), Ok(None));
    let linked = world.lab.at("another-way").display().to_string();
    std::os::unix::fs::symlink(&world.home, &linked).unwrap();
    let refused = reader.read(&link, &slot, &linked, &Request::Probe);
    assert_eq!(refused, Err(Failure::Refused(far::No::Request)));
    assert!(!ran.exists(), "a reader ended through Python's shutdown");
    link.close();
}
