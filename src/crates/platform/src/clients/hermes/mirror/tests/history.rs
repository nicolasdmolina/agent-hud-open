//! What the far database gave, through the one writer (ticket 37): a
//! batch is one transaction, waited for; an older one, one of an old
//! route and one after the writer stopped write nothing.
use super::*;
use crate::clients::hermes::mirror::{Batch, Wrote};
use agent_hud_agents::hermes::far_history::{Cell, Session};
use agent_hud_agents::hermes::mirror::Reads;

/// A far session as the far reader gives it: `input` tokens in one usage
/// row for each of `tasks`, and `print` for its fingerprint.
pub(super) fn far_session(id: &str, input: i64, tasks: &[&str], print: &str) -> Session {
    let text = |text: &str| Cell::Text(text.into());
    let usage = |task: &&str| {
        vec![
            text("claude-test"),
            text("anthropic"),
            text(""),
            text("api"),
            text(task),
            Cell::Int(input),
            Cell::Int(20),
            Cell::Int(0),
            Cell::Int(0),
            Cell::Real(1_760_000_010.0),
            Cell::Real(1_760_000_600.0),
        ]
    };
    Session {
        rowid: 1,
        id: id.to_owned(),
        print: print.to_owned(),
        cells: vec![
            Cell::Real(1_760_000_000.0),
            text(&format!("Far {id}")),
            Cell::Null,
            text("telegram"),
            text("claude-test"),
            text("anthropic"),
            Cell::Null,
            text("api"),
            Cell::Int(input),
            Cell::Int(20),
            Cell::Int(0),
            Cell::Int(0),
            Cell::Real(1_760_000_600.0),
            Cell::Real(1_760_000_600.0),
            Cell::Null,
        ],
        usage: tasks.iter().map(usage).collect(),
    }
}

fn batch(sessions: Vec<Session>, read_ms: i64, route: u64) -> Batch {
    Batch {
        profile: Profile::Default,
        sessions,
        held: None,
        reads: None,
        read_ms,
        route,
    }
}

#[test]
fn a_far_sessions_batch_is_listed_with_its_tokens_and_a_live_row_becomes_durable_in_place() {
    let mut bench = bench("Mac mini");
    bench.lists(&Profile::Default, &[("s1", "From the list")], 1_000, 1);
    assert_eq!(bench.turn(), 1);
    assert_eq!(bench.rows()[0].3, 0);
    let before = bench.dump(&Profile::Default);
    assert!(
        before.contains("hud_rows [Text(\"s1\"), Integer(0), "),
        "{before}"
    );

    // The far database gave it, and one that the live list never had.
    let both = vec![
        far_session("s1", 100, &["", "aux:title"], "print-1"),
        far_session("s2", 7, &[""], "print-2"),
    ];
    let tells = bench.tells();
    assert_eq!(
        bench.writer.history(batch(both, 2_000, 1)),
        Wrote::Written(2)
    );
    // Committed when `history` returns, and core was told once.
    assert_eq!(bench.tells(), tells + 1);
    assert_eq!(bench.turn(), 1);
    let rows = bench.rows();
    let shown: Vec<(&str, &Value, &Value)> = (rows.iter())
        .map(|row| (row.0.as_str(), &row.2, &row.3))
        .collect();
    assert_eq!(
        shown,
        [
            ("default:s1", &json!("Far s1"), &json!(240)),
            ("default:s2", &json!("Far s2"), &json!(27))
        ]
    );
    let after = bench.dump(&Profile::Default);
    assert!(
        after.contains(
            "hud_rows [Text(\"s1\"), Integer(1), Integer(2000), Text(\"print-1\"), Integer(0)]"
        ),
        "{after}"
    );
    // The live row was replaced where it was: the first row still.
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = rusqlite::Connection::open_with_flags(bench.file(&Profile::Default), flags).unwrap();
    let rowids: Vec<(String, i64)> = db
        .prepare("SELECT id, rowid FROM sessions ORDER BY rowid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rowids, [("s1".to_owned(), 1), ("s2".to_owned(), 2)]);

    // A live list that names them again changes nothing, and the ages
    // of the mirror are written without a word to core: it reads none.
    let tells = bench.tells();
    bench.lists(&Profile::Default, &[("s1", "x"), ("s2", "y")], 3_000, 1);
    let ages = Batch {
        held: Some(vec!["s2".into()]),
        reads: Some(Reads {
            contact_ms: Some(3_500),
            ..Reads::default()
        }),
        ..batch(Vec::new(), 3_500, 1)
    };
    assert_eq!(bench.writer.history(ages), Wrote::Written(0));
    assert_eq!(bench.tells(), tells);
    let held: (i64, i64) = db
        .query_row(
            "SELECT (SELECT held FROM hud_rows WHERE session_id='s2'), (SELECT contact_ms FROM hud_reads)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(held, (1, 3_500));
}

#[test]
fn a_batch_that_arrives_after_a_newer_one_for_the_same_session_is_dropped() {
    let mut bench = bench("Mac mini");
    let newer = vec![far_session("s1", 500, &[""], "print-new")];
    assert_eq!(
        bench.writer.history(batch(newer, 2_000, 1)),
        Wrote::Written(1)
    );
    bench.turn();
    let (held, tells) = (bench.dump(&Profile::Default), bench.tells());
    // Read before it, and here after it: nothing of it is written, and
    // a session beside it in the same batch still is.
    let older = vec![
        far_session("s1", 100, &["", "aux:old"], "print-old"),
        far_session("s2", 7, &[""], "print-2"),
    ];
    assert_eq!(
        bench.writer.history(batch(older, 1_000, 1)),
        Wrote::Written(1)
    );
    let now = bench.dump(&Profile::Default);
    for line in held.lines() {
        assert!(now.contains(line), "{line}\n{now}");
    }
    assert!(
        !now.contains("aux:old") && !now.contains("print-old"),
        "{now}"
    );
    bench.turn();
    assert_eq!(bench.rows()[0].3, 520);
    // By itself it writes nothing at all, and core hears nothing.
    let tells_now = bench.tells();
    assert!(tells_now > tells);
    let alone = vec![far_session("s1", 100, &[""], "print-old")];
    assert_eq!(
        bench.writer.history(batch(alone, 1_500, 1)),
        Wrote::Written(0)
    );
    assert_eq!(bench.tells(), tells_now);
    assert_eq!(bench.dump(&Profile::Default), now);
}

#[test]
fn a_batch_read_under_a_route_number_that_is_not_current_writes_nothing() {
    let bench = bench("Mac mini");
    let one = || vec![far_session("s1", 100, &[""], "print-1")];
    // Read under a number the machine no longer has.
    assert_eq!(
        bench.writer.history(batch(one(), 1_000, 0)),
        Wrote::Dropped("route")
    );
    assert!(!bench.file(&Profile::Default).exists());
    // The number goes up between the writer's look and its commit.
    let asked = bench.number.asked.load(Ordering::Acquire);
    bench.number.raise_at.store(asked + 2, Ordering::Release);
    assert_eq!(
        bench.writer.history(batch(one(), 1_000, 1)),
        Wrote::Dropped("route")
    );
    assert!(!bench.file(&Profile::Default).exists());
    assert_eq!(bench.tells(), 0);
    // Under the number as it is now, it is written.
    assert_eq!(
        bench.writer.history(batch(one(), 1_000, 2)),
        Wrote::Written(1)
    );
    assert!(bench
        .log()
        .contains("mirror dropped profile=default why=route history"));
}

#[test]
fn a_batch_that_comes_back_after_the_machine_is_retired_is_dropped() {
    let bench = bench("Mac mini");
    let one = |id: &str| vec![far_session(id, 100, &[""], "print-1")];
    assert_eq!(
        bench.writer.history(batch(one("s1"), 1_000, 1)),
        Wrote::Written(1)
    );
    let held = bench.dump(&Profile::Default);
    bench.writer.stop();
    let stamp = |bench: &Bench| {
        let meta = std::fs::metadata(bench.file(&Profile::Default)).unwrap();
        (meta.len(), meta.modified().unwrap())
    };
    let stopped = stamp(&bench);
    assert_eq!(
        bench.writer.history(batch(one("s2"), 2_000, 1)),
        Wrote::Dropped("over")
    );
    assert_eq!(stamp(&bench), stopped);
    assert_eq!(bench.dump(&Profile::Default), held);
}

#[test]
fn the_refresh_is_given_the_live_sessions_and_once_the_ones_that_left_the_list() {
    let bench = bench("Mac mini");
    let default = Profile::Default;
    assert!(bench.writer.first(&default, 0).is_empty());
    assert!(!bench.writer.working(0));
    bench.lists(&default, &[("a", "A"), ("b", "B")], 1_000, 1);
    assert_eq!(bench.writer.first(&default, 0), ["a", "b"]);
    // Listed is not at work (ticket 61): no poll found a turn.
    assert!(!bench.writer.working(0));
    // `a` left the list: it is given once more, as ended, and then not.
    bench.lists(&default, &[("b", "B"), ("c", "C")], 2_000, 1);
    assert_eq!(bench.writer.first(&default, 0), ["a", "b", "c"]);
    assert_eq!(bench.writer.first(&default, 0), ["b", "c"]);
    // An older list comes late and says nothing.
    bench.lists(&default, &[("z", "Z")], 1_500, 1);
    assert_eq!(bench.writer.first(&default, 0), ["b", "c"]);
    // A list too old to say what is live: its sessions once, as ended.
    assert_eq!(bench.writer.first(&default, 2_001), ["b", "c"]);
    assert!(bench.writer.first(&default, 2_001).is_empty());
    // Another profile's sessions are its own.
    bench.lists(&reports(), &[("r", "R")], 3_000, 1);
    assert_eq!(bench.writer.first(&reports(), 0), ["r"]);
    assert!(bench.writer.first(&default, 0).is_empty());
}

/// Ticket 34's note for this ticket: a mirror that holds a far machine's
/// whole history, as the Mini's does, and a session that begins there.
#[test]
fn a_new_row_is_listed_at_the_first_pass_of_a_mirror_that_holds_a_whole_history() {
    let mut bench = bench("Mac mini");
    // A first sync of two thousand sessions, twenty-five a batch.
    for page in 0..80 {
        let sessions = (0..25)
            .map(|n| {
                far_session(
                    &format!("h{:05}", page * 25 + n),
                    100,
                    &["", "aux:title"],
                    "print",
                )
            })
            .collect();
        assert_eq!(
            bench.writer.history(batch(sessions, 1_000, 1)),
            Wrote::Written(25)
        );
    }
    bench.turn();
    let passes = bench.reads_on();
    assert_eq!(bench.count(), 2_000);
    // A session begins: the next poll lists it.
    let tells = bench.tells();
    bench.lists(&Profile::Default, &[("znew", "Just begun")], 2_000, 1);
    assert_eq!(bench.tells(), tells + 1);
    let committed = *bench.told.at.lock().unwrap().last().unwrap();
    bench.turn();
    let took = committed.elapsed();
    let found = bench.found("Just begun");
    assert_eq!(
        found["items"].as_array().unwrap().len(),
        1,
        "not at the first pass"
    );
    let after = bench.reads_on();
    println!(
        "a mirror of 2,000 stored sessions: first sync read in {passes} more passes; a new row listed {} ms after its commit, at the first pass; {after} passes after it",
        took.as_millis()
    );
    assert_eq!(after, 0);
    assert_eq!(bench.count(), 2_001);
}

#[test]
fn a_session_the_mirror_refuses_is_named_and_logged_and_the_others_of_its_batch_are_written() {
    let mut bench = bench("Mac mini");
    let before = vec![
        far_session("s-good", 100, &[""], "print-1"),
        far_session("s-bad", 50, &[""], "print-2"),
    ];
    assert_eq!(
        bench.writer.history(batch(before, 1_000, 1)),
        Wrote::Written(2)
    );
    assert_eq!(bench.turn(), 1);
    // Two usage rows of one session that name no model, as two far rows
    // whose model is a blob come: one name in the mirror, twice.
    let mut bad = far_session("s-bad", 900, &["", ""], "print-4");
    for row in &mut bad.usage {
        row[0] = Cell::Null;
    }
    let batch_of = |bad: &Session, read_ms| {
        let sessions = vec![
            far_session("s-good", 400, &[""], "print-3"),
            bad.clone(),
            far_session("s-new", 30, &[""], "print-5"),
        ];
        batch(sessions, read_ms, 1)
    };
    let tells = bench.tells();
    assert_eq!(
        bench.writer.history(batch_of(&bad, 2_000)),
        Wrote::Refused(2, vec!["s-bad".to_owned()])
    );
    // The two beside it are committed and core is told of them; the one
    // refused is as it was, counts and fingerprint.
    assert_eq!(bench.tells(), tells + 1);
    assert_eq!(bench.turn(), 1);
    let shown: Vec<(String, Value)> = (bench.rows().into_iter())
        .map(|row| (row.0, row.3))
        .collect();
    assert_eq!(
        shown,
        [
            ("default:s-bad".to_owned(), json!(70)),
            ("default:s-good".to_owned(), json!(420)),
            ("default:s-new".to_owned(), json!(50))
        ]
    );
    let dump = bench.dump(&Profile::Default);
    assert!(
        dump.contains(
            "hud_rows [Text(\"s-bad\"), Integer(1), Integer(1000), Text(\"print-2\"), Integer(0)]"
        ),
        "{dump}"
    );
    // Said once in the log, with the session and SQLite's reason, and
    // not as a write of the mirror that failed.
    let log = bench.log();
    let refused: Vec<&str> = (log.lines())
        .filter(|line| line.contains("mirror refused"))
        .collect();
    assert_eq!(refused.len(), 1, "{log}");
    assert!(
        refused[0].contains("mirror refused profile=default session=s-bad why=UNIQUE"),
        "{log}"
    );
    assert!(!log.contains("mirror failed"), "{log}");
    // The same again is refused again and changes nothing; once the far
    // rows can be told apart it is written like any other.
    assert_eq!(
        bench.writer.history(batch_of(&bad, 3_000)),
        Wrote::Refused(2, vec!["s-bad".to_owned()])
    );
    bad.usage[1][0] = Cell::Text("another-model".into());
    assert_eq!(
        bench.writer.history(batch_of(&bad, 4_000)),
        Wrote::Written(3)
    );
}

#[test]
fn only_a_session_in_a_turn_makes_the_machine_one_that_works() {
    let bench = bench("Mac mini");
    assert!(!bench.writer.working(0));
    // Listed, and many: nothing of that says a session works.
    let many: Vec<(String, &str)> = (0..25).map(|n| (format!("idle-{n:02}"), "Idle")).collect();
    let many: Vec<(&str, &str)> = many
        .iter()
        .map(|(id, title)| (id.as_str(), *title))
        .collect();
    bench.lists(&Profile::Default, &many, 1_000, 1);
    assert_eq!(bench.writer.first(&Profile::Default, 0).len(), 25);
    assert!(!bench.writer.working(0));
    // A poll found one in a turn at 2,000: the machine works for as long
    // as that is fresh, and not after.
    bench.writer.worked(2_000);
    bench.lists(&Profile::Default, &many, 2_000, 1);
    assert!(bench.writer.working(2_000));
    assert!(!bench.writer.working(2_001));
    // An older poll that comes back late does not take it back.
    bench.writer.worked(1_500);
    assert!(bench.writer.working(2_000));
    // Later polls list the same sessions, idle again: still not working.
    bench.lists(&Profile::Default, &many, 9_000, 1);
    assert!(!bench.writer.working(2_001));
}
