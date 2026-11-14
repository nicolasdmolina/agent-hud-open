//! What is not used: a far program that ends badly, writes to standard
//! error or answers in the wrong shape, and a machine with no history
//! reader. Each stand-in holds a made-up "row" that must never be logged.
use super::super::{Failure, Slots};
use super::world::{sessions, settings, World, BIN, PYTHON};
use crate::ssh_link::unix::tests::lab::Lab;
use crate::ssh_link::{End, FarCommand};
use agent_hud_agents::hermes::far_history::{self as far, No, Rejected, Reply, Request};
use std::time::Duration;

const SECRET: &str = "SECRET-ROW-TEXT";

/// A stand-in that answers a probe as the reader does, and anything else
/// with `rest`.
fn answers(rest: &str) -> String {
    format!(
        "cat > /dev/null\ncase \"$*\" in\n*'\"k\":\"probe\"'*) printf '%s\\n' '[\"started\",1]' '[\"end\",\"ok\",0,0,0,0]' ;;\n*) {rest} ;;\nesac"
    )
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_bad_exit_text_on_stderr_and_a_wrong_shape_are_each_rejected_and_logged_without_row_text() {
    let Some(world) = World::start() else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let asked = sessions(&["s1"]);
    let whole = format!(
        "printf '%s\\n' '[\"started\",1]' '[\"s\",1,\"s1\",1.0,\"{SECRET}\",null,null,null,null,null,null,1,2,3,4,null,null,null,\"p\"]' '[\"end\",\"ok\",1,0,0,0]'"
    );
    // The stand-in's whole answer is one the caller takes: the cases
    // below differ from it by one thing each.
    world.stand_in(&answers(&whole));
    let read = reader.read(&link, &slot, &world.home, &asked);
    assert!(
        matches!(&read, Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1),
        "{read:?}"
    );

    let cases: [(&str, String, Failure, &str); 6] = [
        ("a non-zero exit", format!("{whole}; exit 9"), Failure::Exit(9), "exit=9"),
        (
            "text on standard error",
            format!("{whole}; echo '{SECRET} on stderr' >&2"),
            Failure::Stderr,
            "exit=0 stderr=26 B",
        ),
        (
            "a row of the wrong shape",
            format!("printf '%s\\n' '[\"started\",1]' '[\"s\",\"{SECRET}\"]' '[\"end\",\"ok\",1,0,0,0]'"),
            Failure::Rejected(Rejected::Line(2)),
            "exit=0 stderr=0 B",
        ),
        (
            "counts that do not match",
            whole.replace("\"ok\",1,0,0,0", "\"ok\",2,0,0,0"),
            Failure::Rejected(Rejected::Counts),
            "exit=0 stderr=0 B",
        ),
        (
            "a completion line cut off",
            whole.replace(" '[\"end\",\"ok\",1,0,0,0]'", "; printf '%s' '[\"end\",\"ok\",1,0'"),
            Failure::Rejected(Rejected::NoCompletion),
            "exit=0 stderr=0 B",
        ),
        (
            "no completion line at all",
            whole.replace(" '[\"end\",\"ok\",1,0,0,0]'", ""),
            Failure::Rejected(Rejected::NoCompletion),
            "exit=0 stderr=0 B",
        ),
    ];
    for (name, script, failure, logged) in cases {
        world.stand_in(&answers(&script));
        assert_eq!(
            reader.read(&link, &slot, &world.home, &asked),
            Err(failure),
            "{name}"
        );
        let line = world.logged().pop().unwrap();
        assert!(
            line.contains("failed") && line.contains(logged),
            "{name}: {line}"
        );
        assert!(line.ends_with(&failure.to_string()), "{name}: {line}");
    }
    let log = world.logged();
    assert!(log.len() >= 8);
    assert!(log.iter().all(|line| !line.contains("SECRET")), "{log:?}");
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_python_without_sqlite_is_no_history_on_this_machine() {
    let Some(world) = World::start() else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    // The system's Python, with its `sqlite3` module made unimportable.
    world.stand_in(&format!(
        "shift 2\nexec {PYTHON} -I -c 'import sys; sys.modules[\"sqlite3\"] = None; exec(compile(sys.stdin.read(), \"reader\", \"exec\"))' \"$@\""
    ));
    let read = reader.read(&link, &slot, &world.home, &Request::Probe);
    assert_eq!(read, Err(Failure::NoHistory));
    assert_eq!(Failure::NoHistory.to_string(), "no history on this machine");
    let line = world.logged().pop().unwrap();
    assert!(
        line.ends_with("exit=0 stderr=0 B") || line.contains("no history on this machine"),
        "{line}"
    );
    // With the real one, the same machine has a reader.
    world.no_stand_in();
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_machine_with_no_python3_is_no_history_on_this_machine() {
    // A far path with nothing on it but the lab's own folder, which holds
    // a shell and no `python3`.
    let Some(lab) = Lab::start_with(&[("PATH", "{lab}/bin")]) else {
        return;
    };
    std::fs::create_dir_all(lab.at(BIN)).unwrap();
    std::os::unix::fs::symlink("/bin/sh", lab.at(BIN).join("sh")).unwrap();
    let Some(world) = World::on(lab) else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let read = reader.read(&link, &slot, &world.home, &sessions(&["s1"]));
    assert_eq!(read, Err(Failure::NoHistory));
    let line = world.logged().pop().unwrap();
    assert!(
        line.contains("exit=127") && line.ends_with("no history on this machine"),
        "{line}"
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_home_that_is_not_its_own_canonical_path_is_refused() {
    let Some(world) = World::start() else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    // The same folder, reached through a link.
    let linked = world.lab.at("another-way").display().to_string();
    std::os::unix::fs::symlink(&world.home, &linked).unwrap();
    let read = reader.read(&link, &slot, &linked, &Request::Probe);
    assert_eq!(read, Err(Failure::Refused(No::Request)));
    assert_ne!(
        far::mark(&world.machine(), &linked),
        far::mark(&world.machine(), &world.home)
    );
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    link.close();
}

/// The program's own crash, made with a request the caller cannot build:
/// its one line names the kind of error, and nothing of what it held.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_crash_says_its_kind_and_no_row_text() {
    let Some(world) = World::start() else { return };
    let link = world.link();
    let request = format!("{{\"k\":\"prints\",\"after\":0,\"n\":\"{SECRET}\"}}");
    let ran = link
        .run(&FarCommand {
            script: far::SCRIPT,
            args: &[&world.mark(), "5.000", &world.home, &request],
            stdin: Some(far::PROGRAM.as_bytes()),
            limit: Duration::from_secs(30),
            most_output: 64 * 1024,
        })
        .unwrap();
    assert_eq!(ran.end, End::Ended(1));
    assert_eq!(ran.stderr, "far-history: crash ValueError\n");
    let said = String::from_utf8(ran.stdout).unwrap();
    assert!(
        said.starts_with("[\"started\",") && said.lines().count() == 1,
        "{said}"
    );
    // The caller logs that word and the status, and still nothing else.
    let (reader, slots) = (world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    world.stand_in(&answers(
        "printf '%s\\n' '[\"started\",1]'; echo 'far-history: crash ValueError' >&2; exit 1",
    ));
    let read = reader.read(&link, &slot, &world.home, &sessions(&["s1"]));
    assert_eq!(read, Err(Failure::Exit(1)));
    let line = world.logged().pop().unwrap();
    assert!(
        line.contains("exit=1 stderr=30 B crash=ValueError"),
        "{line}"
    );
    // A line that is not the program's own is only counted.
    world.stand_in(&answers(&format!(
        "echo 'far-history: crash {SECRET} and more' >&2; exit 1"
    )));
    assert_eq!(
        reader.read(&link, &slot, &world.home, &sessions(&["s1"])),
        Err(Failure::Exit(1))
    );
    let line = world.logged().pop().unwrap();
    assert!(
        line.contains("exit=1 stderr=44 B ") && !line.contains("crash"),
        "{line}"
    );
    assert!(world.logged().iter().all(|line| !line.contains("SECRET")));
    link.close();
}

/// A far SQLite older than 3.43 cannot say how long a value is without
/// loading it. The reader has no other way to bound what it holds, so it
/// refuses there, with its reason, and opens nothing.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_far_sqlite_too_old_to_measure_a_value_is_refused_by_name() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 1);
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    // The system's Python, saying its SQLite is the last one without it.
    world.stand_in(&format!(
        "shift 2\nexec {PYTHON} -I -c 'import sys, sqlite3; sqlite3.sqlite_version_info = (3, 42, 0); exec(compile(sys.stdin.read(), \"reader\", \"exec\"))' \"$@\""
    ));
    for request in [Request::Probe, sessions(&["s1"])] {
        let read = reader.read(&link, &slot, &world.home, &request);
        assert_eq!(read, Err(Failure::Refused(No::SqliteOld)));
    }
    assert_eq!(
        Failure::Refused(No::SqliteOld).to_string(),
        "the far SQLite is older than 3.43, which cannot bound what it reads"
    );
    let line = world.logged().pop().unwrap();
    assert!(
        line.contains("failed exit=0") && line.ends_with("cannot bound what it reads"),
        "{line}"
    );
    // It said so before it started: no `started` line, and no database.
    let ran = super::world::raw(
        &link,
        &world.home,
        Duration::from_secs(5),
        &sessions(&["s1"]),
    );
    assert_eq!(ran.stdout, b"[\"no\",\"sqlite-old\"]\n");
    // With the SQLite it really has, the same machine is read.
    world.no_stand_in();
    let read = reader.read(&link, &slot, &world.home, &sessions(&["s1"]));
    assert!(
        matches!(&read, Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1),
        "{read:?}"
    );
    link.close();
}

/// A page is never asked from a rowid below zero: not by the caller, and
/// not taken by the far program from anyone else.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_page_from_a_rowid_below_zero_is_refused_on_both_sides() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 1);
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let before = world.logged().len();
    for request in [
        Request::Prints {
            after: -9,
            limit: 1,
            since: None,
        },
        Request::Rowids {
            after: -1,
            limit: 1,
        },
    ] {
        assert_eq!(
            reader.read(&link, &slot, &world.home, &request),
            Err(Failure::Refused(No::Request))
        );
    }
    assert_eq!(
        reader.prints(&link, &slot, &world.home, -9, 1, None),
        Err(Failure::Refused(No::Request))
    );
    // Nothing was sent for them.
    let log = world.logged();
    assert_eq!(log.len(), before + 3);
    assert!(
        log[before..]
            .iter()
            .all(|line| line.ends_with("not asked: a rowid below zero")),
        "{log:?}"
    );
    // The far program, asked all the same, refuses before it starts.
    for request in [
        "{\"k\":\"prints\",\"after\":-9,\"n\":1}",
        "{\"k\":\"rowids\",\"after\":-1,\"n\":1}",
    ] {
        let ran = link
            .run(&FarCommand {
                script: far::SCRIPT,
                args: &[&world.mark(), "5.000", &world.home, request],
                stdin: Some(far::PROGRAM.as_bytes()),
                limit: Duration::from_secs(30),
                most_output: 64 * 1024,
            })
            .unwrap();
        assert_eq!(
            (ran.end, ran.stdout.as_slice()),
            (End::Ended(0), &b"[\"no\",\"request\"]\n"[..]),
            "{request}"
        );
    }
    // From zero, the page is read.
    let page = reader
        .prints(&link, &slot, &world.home, 0, 1, None)
        .unwrap();
    assert_eq!(page.prints.len(), 1);
    link.close();
}
