//! What is not used: a far program that ends badly, writes to standard
//! error or answers in the wrong shape, and a machine with no history
//! reader. Each stand-in holds a made-up "row" that must never be logged.
use super::super::{Failure, Slots};
use super::world::{machine, sessions, settings, World, PYTHON};
use crate::ssh_link::unix::tests::lab::{Lab, BIN};
use agent_hud_agents::hermes::far_history::{Rejected, Reply, Request};

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
    let slot = slots.of(&machine());
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
    let slot = slots.of(&machine());
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
    let Some(lab) = Lab::start_on("") else { return };
    std::os::unix::fs::symlink("/bin/sh", lab.at(BIN).join("sh")).unwrap();
    let Some(world) = World::on(lab) else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let read = reader.read(&link, &slot, &world.home, &sessions(&["s1"]));
    assert_eq!(read, Err(Failure::NoHistory));
    let line = world.logged().pop().unwrap();
    assert!(
        line.contains("exit=127") && line.ends_with("no history on this machine"),
        "{line}"
    );
    link.close();
}
