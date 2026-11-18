//! The refresh across what happens to a machine (ticket 37): a batch the
//! far side cut or cannot give, a new address, retirement, and the one
//! reader slot a machine keeps through all of it. In the lab of
//! `history.rs`.
use super::{
    here,
    history::{mirrored, now, plan, rows, session, used, Site, OLD},
    ssh, until,
};
use crate::clients::hermes::{
    history::{Plan, Without},
    machines::words,
    mirror::{Batch, Wrote},
};
use agent_hud_agents::hermes::{
    far_history::{refresh, Cell, Session, MARK},
    gateway::Profile,
};
use rusqlite::Connection;
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

/// Puts a stand-in for `python3` first on the far path of the site's
/// host: `script` is run for a history reader, with the reader's own
/// arguments, and everything else gets the system's Python.
fn stand_in(site: &Site, script: &str) {
    let bin = site.fleet.hosts[0].lab.at("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let text = format!(
        "#!/bin/sh\ncase \"$4\" in {MARK}*)\n{script}\n;; esac\nexec /usr/bin/python3 \"$@\"\n"
    );
    std::fs::write(bin.join("python3"), text).unwrap();
    let mode = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(bin.join("python3"), mode).unwrap();
}

fn no_stand_in(site: &Site) {
    let _ = std::fs::remove_file(site.fleet.hosts[0].lab.at("bin").join("python3"));
}

/// A stand-in that runs the real reader from a process that does not
/// carry its mark, so the reader still finds itself alone: its arguments
/// go through a file. `then` is shell that runs there, with the reader's
/// arguments as its own (`$7` is the request).
fn wrapped(site: &Site, then: &str) {
    let lab = &site.fleet.hosts[0].lab;
    let (args, wrapper) = (lab.at("reader-args"), lab.at("reader.sh"));
    let text = format!(
        "set --\nwhile IFS= read -r line; do set -- \"$@\" \"$line\"; done < '{}'\n{then}\nexec /usr/bin/python3 \"$@\"\n",
        args.display()
    );
    std::fs::write(&wrapper, text).unwrap();
    stand_in(
        site,
        &format!(
            "printf '%s\\n' \"$@\" > '{}'\nexec /bin/sh '{}'",
            args.display(),
            wrapper.display()
        ),
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_rejected_or_skipped_batch_writes_nothing_and_the_session_keeps_its_old_rows() {
    let seed = |db: &Connection| {
        for id in ["s-cut", "s-many", "s-fine"] {
            session(db, id, OLD);
            used(db, id, "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    site.sync();
    site.read();
    let held = rows(&site.mirror());
    let before = site.total();

    // One grows past the usage rows the reader accepts: it is skipped
    // whole. One grows while the far side cuts every sessions answer
    // short of its last line: that batch is rejected whole.
    for row in 0..501 {
        used(&site.db, "s-many", "model-a", &format!("aux:{row}"), 1);
    }
    site.db
        .execute(
            "UPDATE session_model_usage SET input_tokens=900 WHERE session_id='s-cut'",
            [],
        )
        .unwrap();
    let cut = site.fleet.hosts[0].lab.at("cut");
    std::fs::write(&cut, "").unwrap();
    wrapped(
        &site,
        &format!(
            "case \"$7\" in *'\"k\":\"sessions\"'*) if [ -e '{}' ]; then /usr/bin/python3 \"$@\" | sed '$d'; exit 0; fi;; esac",
            cut.display()
        ),
    );
    for _ in 0..3 {
        site.history.tick();
    }
    let reads = site.reads();
    println!("{}", reads.join("\n"));
    assert!(
        reads
            .iter()
            .any(|read| read.contains("answer rejected: no completion line")),
        "{reads:?}"
    );
    // Nothing of either was written: every row as it was, core told of
    // nothing, the ledger where it was.
    assert_eq!(rows(&site.mirror()), held);
    assert!(!site.fleet.asked());
    assert_eq!(site.total(), before);

    // The far side answers whole again: the cut session is written, the
    // one with too many rows is still skipped, with its old rows.
    std::fs::remove_file(&cut).unwrap();
    let within = refresh::sweep_pages(3, plan().page) + 2;
    super::history::ticks_until(&site, within, || {
        mirrored::<i64>(
            &site.mirror(),
            "SELECT input_tokens FROM session_model_usage WHERE session_id='s-cut'",
        ) == [900]
    });
    site.read();
    assert_eq!(site.total() - before, 800);
    let many: Vec<i64> = mirrored(
        &site.mirror(),
        "SELECT COUNT(*) FROM session_model_usage WHERE session_id='s-many'",
    );
    assert_eq!(many, [1]);
    assert!(site
        .reads()
        .iter()
        .any(|read| read.contains("skips one session: more usage rows")));
    // And it is not asked for again while it stays as it is.
    let asked = |site: &Site| {
        site.reads()
            .iter()
            .filter(|read| read.starts_with("sessions "))
            .count()
    };
    let before = asked(&site);
    for _ in 0..3 {
        site.history.tick();
    }
    assert_eq!(asked(&site), before, "{:?}", site.reads());
    no_stand_in(&site);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn history_goes_on_after_a_same_machine_address_change() {
    // A page of seven and four reads a tick: the first tick reads two
    // pages of the thirty sessions and stops mid-sweep.
    let plan = Plan {
        reads: 4,
        page: 7,
        ..plan()
    };
    let thirty = |db: &Connection| {
        for n in 0..30 {
            session(db, &format!("s-{n:02}"), OLD + f64::from(n));
            used(db, &format!("s-{n:02}"), "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    assert_eq!(site.history.tick().reads, 4);
    let count =
        |site: &Site| -> i64 { mirrored(&site.mirror(), "SELECT COUNT(*) FROM sessions")[0] };
    let place = site.history.sweep(&Profile::Default);
    assert_eq!((place, count(&site)), ((14, 0), 14));
    let stored: Vec<i64> = mirrored(&site.mirror(), "SELECT sweep_cursor FROM hud_reads");
    assert_eq!(stored, [14]);
    let inode = std::fs::metadata(site.mirror()).unwrap().ino();

    // The entry's address is edited to another name of the same machine.
    site.fleet.write(&[here(), ssh("a", "Mini", "m0b")]);
    until("the route number goes up", || site.fleet.number() == 2);
    assert_eq!(site.fleet.connected("Mini"), site.key);
    // The same refresh, with its sweep where it was, on the same mirror.
    let now = site.fleet.book().history_of("a").expect("its refresh");
    assert!(Arc::ptr_eq(&now, &site.history));
    assert_eq!(site.history.sweep(&Profile::Default), place);
    // The next tick that has a link reads on from there, and writes.
    let mut ticked = site.history.tick();
    until("a tick over the new route", || {
        if ticked.reads == 0 {
            ticked = site.history.tick();
        }
        ticked.reads > 0
    });
    assert_eq!(count(&site), 28, "{:?}", site.reads());
    assert_eq!(site.history.sweep(&Profile::Default), (28, 0));
    assert_eq!(std::fs::metadata(site.mirror()).unwrap().ino(), inode);
    // No page was read twice: the sweep never began anew.
    let pages: Vec<String> = (site.reads().into_iter())
        .filter(|read| read.starts_with("prints ok"))
        .collect();
    assert_eq!(pages.len(), 4, "{pages:?}");
    site.sync();
    assert_eq!(count(&site), 30);
    assert_eq!(site.read().len(), 30);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_page_that_takes_the_far_side_too_long_is_asked_for_again_smaller() {
    // A page of twenty rows takes the far side eight seconds to begin,
    // and this side waits four for an answer.
    let mut plan = Plan { page: 20, ..plan() };
    plan.reader.local_wait = Duration::from_secs(4);
    let thirty = |db: &Connection| {
        for n in 0..30 {
            session(db, &format!("s-{n:02}"), OLD + f64::from(n));
            used(db, &format!("s-{n:02}"), "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    wrapped(&site, "case \"$7\" in *'\"n\":20'*) sleep 8;; esac");
    let ticked = site.history.tick();
    assert_eq!(ticked.reads, 1, "{:?}", site.reads());
    assert!(site
        .reads()
        .iter()
        .any(|read| read.contains("cut off at the local time limit")));
    assert!(!site.mirror().exists());
    // From the next tick on the page is half the size, and the sweep
    // goes through: a slow database slows it and does not stop it.
    site.sync();
    let pages: Vec<String> = (site.reads().into_iter())
        .filter(|read| read.starts_with("prints ok"))
        .collect();
    println!("{pages:?}");
    assert!(
        pages.len() >= 3 && pages.iter().all(|page| !page.contains("seen=20")),
        "{pages:?}"
    );
    assert!(
        pages.iter().any(|page| page.contains("seen=10")),
        "{pages:?}"
    );
    let count: Vec<i64> = mirrored(&site.mirror(), "SELECT COUNT(*) FROM sessions");
    assert_eq!(count, [30]);
    no_stand_in(&site);
    assert_eq!(site.read().len(), 30);
}

/// A far session as the far reader gives it, for a batch put on a
/// writer's queue by hand.
fn late(id: &str) -> Session {
    let text = |text: &str| Cell::Text(text.into());
    let mut cells = vec![Cell::Null; 15];
    (cells[0], cells[1], cells[8]) = (Cell::Real(1_760_000_000.0), text("Late"), Cell::Int(5_000));
    Session {
        rowid: 99,
        id: id.to_owned(),
        print: "late".into(),
        cells,
        usage: Vec::new(),
    }
}

/// The far processes that carry a history reader's mark of the machine
/// `key`, and which of them have `database` open: counted on the far
/// side, from its process list and its open files.
fn far_readers(key: &str, database: &Path) -> (Vec<u32>, usize) {
    let mark = format!("{MARK}{key}:");
    let listed = Command::new("/bin/ps")
        .args(["-A", "-ww", "-o", "pid=,args="])
        .output()
        .expect("ps");
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
    let marked = |line: &str| {
        let (pid, args) = line.trim_start().split_once(' ')?;
        let carries = args.split_whitespace().any(|word| word.starts_with(&mark));
        carries.then(|| pid.parse::<u32>().ok()).flatten()
    };
    let pids: Vec<u32> = listed.lines().filter_map(marked).collect();
    if pids.is_empty() {
        return (pids, 0);
    }
    let of: Vec<String> = pids.iter().map(u32::to_string).collect();
    let files = Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &of.join(","), "-Fpn"])
        .output()
        .expect("lsof");
    let files = String::from_utf8_lossy(&files.stdout).into_owned();
    let name = format!("n{}", database.display());
    let mut open = std::collections::HashSet::new();
    let mut pid = String::new();
    for line in files.lines() {
        match line.strip_prefix('p') {
            Some(of) => pid = of.to_owned(),
            None if line == name => {
                open.insert(pid.clone());
            }
            None => {}
        }
    }
    (pids, open.len())
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn history_stops_when_the_machine_is_retired() {
    let seed = |db: &Connection| {
        session(db, "s-one", OLD);
        used(db, "s-one", "model-a", "", 100);
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    site.sync();
    assert_eq!(site.read().len(), 1);
    let (held, total) = (rows(&site.mirror()), site.total());
    let database = site.fleet.hosts[0].root.join("state.db");
    let (history, writer) = (site.history.clone(), site.fleet.writer("a"));
    // More to read, and a far side that takes three seconds to start a
    // reader: a tick is out when the machine is retired.
    session(&site.db, "s-after", now() - 60.0);
    used(&site.db, "s-after", "model-a", "", 700);
    wrapped(&site, "sleep 3");
    let ticking = history.clone();
    let out = std::thread::spawn(move || ticking.tick());
    until("a read is out", || {
        site.fleet.log().matches(" history ").count() > site.reads().len()
            || !far_readers(&site.key, &database).0.is_empty()
            || std::fs::metadata(site.fleet.hosts[0].lab.at("reader-args")).is_ok()
    });
    let reads = site.reads().len();
    site.fleet.write(&[here()]);
    until("the machine is retired", || {
        site.fleet.book().far().is_empty()
    });
    let ticked = out.join().unwrap();
    println!(
        "the tick that was out: {ticked:?}; {:?}",
        &site.reads()[reads..]
    );

    // No read begins from now on: a tick reads nothing.
    let reads = site.reads().len();
    assert_eq!(history.tick().reads, 0);
    assert_eq!(history.tick().reads, 0);
    assert_eq!(site.reads().len(), reads);
    // A batch that comes back after that is dropped, whichever route
    // number it was read under.
    for route in [1, 2] {
        let batch = Batch {
            profile: Profile::Default,
            sessions: vec![late("s-late")],
            held: None,
            reads: None,
            read_ms: crate::hooks::now_ms(),
            route,
        };
        assert_eq!(writer.history(batch), Wrote::Dropped("over"));
    }
    // The mirror is as it was, and stays as history: its session is
    // still listed, and nothing more was counted.
    assert_eq!(rows(&site.mirror()), held);
    site.fleet.turn();
    assert_eq!(site.fleet.far_rows().len(), 1);
    assert_eq!(site.total(), total);
    no_stand_in(&site);
    until("no far reader is left", || {
        far_readers(&site.key, &database).0.is_empty()
    });
}

/// Makes every fingerprint read of the far database never end, so a
/// reader lives until its own deadline with the database open
/// (`far_history/unix/tests/world.rs`, `ENDLESS`).
const ENDLESS: &str = "DROP TABLE session_model_usage;
    CREATE VIEW session_model_usage AS WITH RECURSIVE spin(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM spin)
    SELECT 'endless' AS session_id, 'model' AS model FROM spin WHERE n < 0;
    INSERT OR IGNORE INTO sessions (id, source, started_at) VALUES ('endless', 'lab', 1.0);";

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_refresh_uses_the_machines_one_reader_slot() {
    let seed = |db: &Connection| db.execute_batch(ENDLESS).unwrap();
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    let database = site.fleet.hosts[0].root.join("state.db");
    let key = site.key.clone();
    // The far side, looked at throughout: how many readers of the
    // machine there are, and how many have the database open.
    let stop = Arc::new(AtomicBool::new(false));
    let (stopped, watched, at) = (stop.clone(), key.clone(), database.clone());
    let watch = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while !stopped.load(Ordering::Acquire) {
            let (readers, open) = far_readers(&watched, &at);
            seen.push((readers.len(), open));
            std::thread::sleep(Duration::from_millis(15));
        }
        seen
    });
    let machine = site.history.slot().machine().clone();
    let slot = site.fleet.book().slots().of(&machine);
    assert!(Arc::ptr_eq(site.history.slot(), &slot));
    // A tick whose read is out on the far side, with the database open.
    let out = |site: &Site| {
        let ticking = site.fleet.book().history_of("a").expect("its refresh");
        let tick = std::thread::spawn(move || ticking.tick());
        until("a reader has the database open", || {
            far_readers(&site.key, &database).1 == 1
        });
        tick
    };
    // A tick as soon as there is a link again: it asks the same slot.
    let again = |site: &Site| {
        let history = site.fleet.book().history_of("a").expect("its refresh");
        let mut ticked = history.tick();
        until("a tick that read", || {
            if ticked.reads == 0 {
                ticked = history.tick();
            }
            ticked.reads > 0
        });
    };

    // After a reconnect: the link's master is ended while a read is out.
    let tick = out(&site);
    let link = site
        .fleet
        .book()
        .route_of("a")
        .unwrap()
        .route
        .link()
        .unwrap()
        .0;
    let master = link.master_pid().expect("the master") as libc::pid_t;
    // SAFETY: a signal to the `ssh` master of this test's own lab.
    unsafe { libc::kill(master, libc::SIGKILL) };
    tick.join().unwrap();
    again(&site);
    assert!(Arc::ptr_eq(
        site.fleet.book().history_of("a").unwrap().slot(),
        &slot
    ));

    // After a route change: another address of the same machine.
    let tick = out(&site);
    site.fleet.write(&[here(), ssh("a", "Mini", "m0b")]);
    until("the route number goes up", || site.fleet.number() == 2);
    assert_eq!(site.fleet.connected("Mini"), key);
    tick.join().unwrap();
    again(&site);
    assert!(Arc::ptr_eq(
        site.fleet.book().history_of("a").unwrap().slot(),
        &slot
    ));

    // After the machine is rebuilt: retired, and added again at once.
    let probes = |site: &Site| {
        let probe = |read: &&String| read.contains("probes first");
        site.reads().iter().filter(probe).count()
    };
    let probed = probes(&site);
    let old = site.fleet.book().history_of("a").unwrap();
    let tick = out(&site);
    site.fleet.write(&[here()]);
    until("the machine is retired", || {
        site.fleet.book().far().is_empty()
    });
    site.fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    assert_eq!(site.fleet.connected("Mini"), key);
    let new = site.fleet.book().history_of("a").expect("its refresh");
    assert!(
        !Arc::ptr_eq(&new, &old),
        "a machine made anew has a refresh of its own"
    );
    // And the slot it had: asked of the book's, never made.
    assert!(Arc::ptr_eq(new.slot(), &slot));
    assert!(Arc::ptr_eq(&site.fleet.book().slots().of(&machine), &slot));
    tick.join().unwrap();
    again(&site);

    stop.store(true, Ordering::Release);
    let seen = watch.join().unwrap();
    let most = |part: fn(&(usize, usize)) -> usize| seen.iter().map(part).max().unwrap_or(0);
    let open = seen.iter().filter(|look| look.1 == 1).count();
    println!(
        "far side: {} looks; at most {} reader process(es), at most {} with the database open; open in {open} looks\n{}",
        seen.len(),
        most(|look| look.0),
        most(|look| look.1),
        site.reads().join("\n")
    );
    // One reader with the database open, or none, at every look; and a
    // second process only ever as a probe that opened nothing.
    assert_eq!(most(|look| look.1), 1);
    assert!(most(|look| look.0) <= 2);
    assert!(open >= 3);
    // Retiring the machine left nothing known of its reader: the first
    // read of the machine made anew probed, and waited on no countdown.
    assert!(probes(&site) > probed, "{:?}", site.reads());
    drop(site.db);
    until("no far reader is left", || {
        far_readers(&key, &database).0.is_empty()
    });
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_machine_with_no_history_reader_says_so_in_its_status() {
    let seed = |db: &Connection| {
        session(db, "s-one", OLD);
        used(db, "s-one", "model-a", "", 100);
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    // The far Python has no `sqlite3` for a history reader. The relay
    // runs as it did: the machine is followed, and has its cards.
    stand_in(
        &site,
        "shift 3\nexec /usr/bin/python3 -I -c 'import sys; sys.modules[\"sqlite3\"] = None; exec(compile(sys.stdin.read(), \"reader\", \"exec\"))' \"$@\"",
    );
    assert!(site.history.tick().reads > 0);
    let said = words::no_history(Without::Python);
    let mut row = serde_json::Value::Null;
    until("the status says there is no history", || {
        row = site.fleet.stands("Mini", "connected");
        row["note"] == said.as_str()
    });
    assert!(
        row.get("lastContactAt").is_none() && row.get("historyCheckedAt").is_none(),
        "{row}"
    );
    assert!(site.read().is_empty());
    assert!(!site.mirror().exists());
    // With a Python that can read it, the machine has its history and
    // the note goes.
    no_stand_in(&site);
    site.sync();
    until("the note goes", || {
        row = site.fleet.stands("Mini", "connected");
        row.get("note").is_none_or(serde_json::Value::is_null)
    });
    assert!(row["historyCheckedAt"].is_string(), "{row}");
    assert_eq!(site.read().len(), 1);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn what_was_read_under_an_old_route_number_is_not_written() {
    let seed = |db: &Connection| {
        for id in ["s-one", "s-two"] {
            session(db, id, OLD);
            used(db, id, "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    // The far side takes two seconds over a read of sessions. While
    // that read is out, the machine's route number goes up, and the way
    // behind it is put back as it was: the link stands, so the read
    // comes back whole, under a number that is the machine's no more.
    wrapped(
        &site,
        "case \"$7\" in *'\"k\":\"sessions\"'*) sleep 2;; esac",
    );
    let ticking = site.history.clone();
    let out = std::thread::spawn(move || ticking.tick());
    let args = site.fleet.hosts[0].lab.at("reader-args");
    until("the read of sessions is out", || {
        std::fs::read_to_string(&args).is_ok_and(|args| args.contains("\"k\":\"sessions\""))
    });
    let switch = site.fleet.book().switch_of("a").expect("its switch");
    let way: Arc<dyn crate::clients::hermes::route::Route> =
        site.fleet.book().route_of("a").unwrap().route.clone();
    assert_eq!(switch.raise(), 2);
    switch.install(way);
    out.join().unwrap();
    let reads = site.reads();
    println!("{}", reads.join("\n"));
    let whole = |read: &String| read.starts_with("sessions ok") && read.contains("sessions=2");
    assert!(reads.iter().any(whole), "{reads:?}");
    // Nothing of it was written, and the writer said why.
    assert!(!site.mirror().exists());
    let dropped = "mirror dropped profile=default why=route history";
    assert!(site.fleet.log().contains(dropped));
    assert!(site.read().is_empty());
    // They were not taken for done: the next tick reads them under the
    // number as it is, and writes.
    no_stand_in(&site);
    site.sync();
    assert_eq!(site.read().len(), 2);
    assert_eq!(site.fleet.number(), 2);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_relaunch_goes_on_from_the_sweeps_place_in_the_mirror() {
    let plan = Plan {
        reads: 4,
        page: 7,
        ..plan()
    };
    let thirty = |db: &Connection| {
        for n in 0..30 {
            session(db, &format!("s-{n:02}"), OLD + f64::from(n));
            used(db, &format!("s-{n:02}"), "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan, thirty) else {
        return;
    };
    assert_eq!(site.history.tick().reads, 4);
    assert_eq!(site.history.sweep(&Profile::Default), (14, 0));
    // Client hooks off and on again, as a relaunch: a refresh made anew.
    site.fleet.stop();
    site.fleet.start_at(crate::clients::hermes::Pace {
        history: plan,
        ..super::pace()
    });
    assert_eq!(site.fleet.connected("Mini"), site.key);
    let history = site.fleet.book().history_of("a").expect("its refresh");
    assert!(!Arc::ptr_eq(&history, &site.history));
    let mut ticked = history.tick();
    until("a tick that read", || {
        if ticked.reads == 0 {
            ticked = history.tick();
        }
        ticked.reads > 0
    });
    // It read on from the fourteenth row, where the mirror says the
    // sweep had got to, and not from the start.
    let count: Vec<i64> = mirrored(&site.mirror(), "SELECT COUNT(*) FROM sessions");
    assert_eq!(count, [28], "{:?}", site.reads());
    assert_eq!(history.sweep(&Profile::Default), (28, 0));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn each_ticks_held_read_marks_the_sessions_the_desktop_app_holds() {
    let seed = |db: &Connection| {
        for id in ["s-desktop", "s-telegram", "s-other"] {
            session(db, id, OLD);
        }
    };
    let Some(site) = Site::with(plan(), seed) else {
        return;
    };
    let runtime = site.fleet.hosts[0].root.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap(); // plain directory
    let registry = |entries: &str| {
        let text = format!("{{\"entries\": [{entries}]}}");
        std::fs::write(runtime.join("active_sessions.json"), text).unwrap();
    };
    registry(
        r#"{"session_id": "s-desktop", "surface": "desktop", "pid": 1},
           {"session_id": "s-telegram", "surface": "telegram", "pid": 2}"#,
    );
    site.sync();
    let held = |site: &Site| -> Vec<String> {
        let sql = "SELECT session_id FROM hud_rows WHERE held<>0 ORDER BY 1";
        mirrored(&site.mirror(), sql)
    };
    site.history.tick();
    assert_eq!(held(&site), ["s-desktop"]);
    // The desktop app lets go of one and takes another.
    registry(r#"{"session_id": "s-other", "surface": "desktop", "pid": 1}"#);
    site.history.tick();
    assert_eq!(held(&site), ["s-other"]);
    // A registry that is gone holds none.
    std::fs::remove_file(runtime.join("active_sessions.json")).unwrap();
    site.history.tick();
    assert!(held(&site).is_empty());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_refresh_ticks_by_the_clock_and_ends_with_its_machine() {
    // By the clock, a fifth of a second apart.
    let plan = Plan {
        clock: true,
        first: Duration::from_millis(100),
        live: Duration::from_millis(200),
        idle: Duration::from_millis(200),
        ..plan()
    };
    let seed = |db: &Connection| {
        for id in ["s-one", "s-two", "s-three"] {
            session(db, id, OLD);
            used(db, id, "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan, seed) else {
        return;
    };
    // Nobody ticks: the sessions are read all the same.
    until("the history is listed", || site.read().len() == 3);
    // And a session that ends later, at a tick of its own.
    session(&site.db, "s-later", now() - 30.0);
    used(&site.db, "s-later", "model-a", "", 50);
    until("the later one is listed", || site.read().len() == 4);

    // Retired: its thread ends, which held the refresh, and nothing
    // more is read.
    site.fleet.write(&[here()]);
    until("the machine is retired", || {
        site.fleet.book().far().is_empty()
    });
    until("the refresh's thread has ended", || {
        Arc::strong_count(&site.history) == 1
    });
    let reads = site.reads().len();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(site.reads().len(), reads);
    assert_eq!(site.fleet.far_rows().len(), 4);
}
