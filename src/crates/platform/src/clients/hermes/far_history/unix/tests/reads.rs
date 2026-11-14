//! Each kind of read against the lab database, and what is too large.
use super::super::{Failure, Hold, Skip, Slots};
use super::world::{direct, machine, sessions, settings, sort, World};
use agent_hud_agents::hermes::far_history::{self as far, Reply, Request, MOST_USAGE_ROWS};
use std::{fs, time::Instant};

fn owned(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn each_kind_of_read_returns_the_labs_rows() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    for (id, usage) in [("s-one", 0), ("s-two", 1), ("s-three", 3), ("s-four", 40)] {
        world.add_session(&db, id, usage);
    }
    let held = r#"[{"session_id":"s-two","surface":"desktop","pid":4242}]"#;
    fs::create_dir_all(format!("{}/runtime", world.home)).unwrap();
    fs::write(format!("{}/runtime/active_sessions.json", world.home), held).unwrap();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    assert_eq!(slot.hold(), Hold::StartUnknown);

    // Fingerprints, a page at a time by rowid.
    let page = reader
        .prints(&link, &slot, &world.home, 0, 3, None)
        .unwrap();
    let ids = |page: &far::Prints| -> Vec<String> {
        page.prints.iter().map(|print| print.id.clone()).collect()
    };
    assert_eq!(ids(&page), ["s-one", "s-two", "s-three"]);
    assert_eq!((page.seen, page.last, page.ended), (3, 3, false));
    let rest = reader
        .prints(&link, &slot, &world.home, page.last, 3, None)
        .unwrap();
    assert_eq!(ids(&rest), ["s-four"]);
    assert_eq!((rest.seen, rest.last, rest.ended), (1, 4, true));
    assert_eq!(slot.hold(), Hold::Free);

    // The sessions active since a time: one whose own stamp moved, and
    // one that only got a new auxiliary usage row, which leaves the
    // `sessions` row as it was.
    db.execute(
        "UPDATE sessions SET last_activity_at=1790000000.5 WHERE id='s-three'",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_model_usage (session_id, model, task, input_tokens, first_seen, last_seen)
         VALUES ('s-one', 'aux-model', 'aux:compression', 5, 1790000001.0, 1790000001.0)",
        [],
    )
    .unwrap();
    let since = reader
        .prints(&link, &slot, &world.home, 0, 100, Some(1_789_999_999.0))
        .unwrap();
    assert_eq!(ids(&since), ["s-one", "s-three"]);
    assert_eq!((since.seen, since.last, since.ended), (4, 4, true));

    // Sessions by id: each row and every usage row equal a read made here,
    // straight from the same database, field for field.
    let asked = owned(&["s-four", "s-one", "nobody", "s-two", "s-three", "s-one"]);
    let read = reader.sessions(&link, &slot, &world.home, &asked);
    assert_eq!(read.stopped, None);
    assert_eq!(read.gone, ["nobody"]);
    assert!(read.skipped.is_empty() && read.unread.is_empty());
    let came: Vec<&str> = read
        .read
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(came, ["s-four", "s-one", "s-two", "s-three"]);
    let own = far::local_prints(&db, 0, 100).unwrap();
    for mut session in read.read {
        let (cells, usage) = direct(&db, &session.id);
        assert_eq!(session.cells, cells, "{}", session.id);
        sort(&mut session.usage);
        assert_eq!(session.usage, usage, "{}", session.id);
        let print = own.iter().find(|print| print.id == session.id).unwrap();
        assert_eq!((session.rowid, &session.print), (print.rowid, &print.print));
        // The task and the base URL of each usage row are its own.
        for row in &session.usage {
            assert!(
                matches!(&row[2], far::Cell::Text(url) if url.is_empty() || url.starts_with("https://openrouter.ai/api/v"))
            );
        }
        let tasks: Vec<&far::Cell> = session.usage.iter().map(|row| &row[4]).collect();
        if session.id == "s-one" {
            assert_eq!(tasks, [&far::Cell::Text("aux:compression".into())]);
        }
    }
    let usage: i64 = db
        .query_row("SELECT COUNT(*) FROM session_model_usage", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(usage, 45);

    // The held sessions, as the file has them.
    assert_eq!(
        reader.held(&link, &slot, &world.home).unwrap().as_deref(),
        Some(held)
    );
    link.close();
}

/// The far program's fingerprint and the HUD's own, for every row of one
/// database: stamps that are missing, whole and fractional, counts that
/// are missing, and sessions with and without usage rows.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_programs_fingerprint_is_the_huds_own() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    for (id, usage) in [("a", 0), ("b", 2), ("c", 7), ("d", 1)] {
        world.add_session(&db, id, usage);
    }
    db.execute_batch(
        "UPDATE sessions SET ended_at=NULL, last_activity_at=NULL, input_tokens=NULL WHERE id='a';
         UPDATE sessions SET ended_at=1760000900, last_activity_at=1760000901 WHERE id='b';
         UPDATE session_model_usage SET last_seen=NULL WHERE session_id='d';
         UPDATE session_model_usage SET input_tokens=9007199254740993 WHERE session_id='c' AND task='';",
    )
    .unwrap();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let far = reader
        .prints(&link, &slot, &world.home, 0, 100, None)
        .unwrap();
    let own = far::local_prints(&db, 0, 100).unwrap();
    assert_eq!(own.len(), 4);
    assert_eq!(far.prints, own);
    for print in &own {
        println!("  {} {}", print.id, print.print);
    }
    // And the one a sessions read carries is the same again.
    let read = reader.sessions(&link, &slot, &world.home, &owned(&["a", "b", "c", "d"]));
    let carried: Vec<&String> = read.read.iter().map(|session| &session.print).collect();
    assert_eq!(
        carried,
        own.iter().map(|print| &print.print).collect::<Vec<_>>()
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_batch_over_the_output_bound_is_too_large_and_the_caller_halves_it() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    let names: Vec<String> = (0..8).map(|number| format!("s-{number}")).collect();
    for id in &names {
        world.add_session(&db, id, 100);
    }
    world.add_session(&db, "s-huge", 499);
    let mut settings = settings();
    settings.most_output = 64 * 1024;
    let (link, reader, slots) = (world.link(), world.reader(settings), Slots::new());
    let slot = slots.of(&machine());

    // Asked for all at once, the far side answers "too large" and no row.
    let all = Request::Sessions { ids: names.clone() };
    assert_eq!(
        reader.read(&link, &slot, &world.home, &all),
        Ok(Reply::TooLarge)
    );
    let line = world.logged().pop().unwrap();
    println!("  {line}");
    let bytes: usize = line
        .split(" B")
        .next()
        .and_then(|start| start.rsplit(' ').next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(bytes < 100, "{line}");

    // The caller halves it and gets every session whole.
    let mut asked = names.clone();
    asked.push("s-huge".into());
    let read = reader.sessions(&link, &slot, &world.home, &asked);
    assert_eq!(read.stopped, None);
    let came: Vec<&String> = read.read.iter().map(|session| &session.id).collect();
    assert_eq!(came, names.iter().collect::<Vec<_>>());
    for session in &read.read {
        assert_eq!(session.usage.len(), 100);
    }
    // One session that does not fit by itself is skipped, never cut.
    assert_eq!(read.skipped, [("s-huge".to_owned(), Skip::TooLarge)]);
    let log = world.logged();
    assert!(
        log.iter()
            .any(|line| line.contains("too large for 9 sessions: asks for half")),
        "{log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("skips one session: too large for one read")),
        "{log:?}"
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_session_with_501_usage_rows_is_skipped_whole() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    world.add_session(&db, "s-most", MOST_USAGE_ROWS);
    world.add_session(&db, "s-over", MOST_USAGE_ROWS + 1);
    world.add_session(&db, "s-small", 2);
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let read = reader.sessions(
        &link,
        &slot,
        &world.home,
        &owned(&["s-over", "s-most", "s-small"]),
    );
    assert_eq!(read.stopped, None);
    assert_eq!(read.skipped, [("s-over".to_owned(), Skip::UsageRows)]);
    assert_eq!(
        Skip::UsageRows.reason(),
        "more usage rows than the reader accepts"
    );
    let came: Vec<(&str, usize)> = read
        .read
        .iter()
        .map(|session| (session.id.as_str(), session.usage.len()))
        .collect();
    assert_eq!(came, [("s-most", MOST_USAGE_ROWS), ("s-small", 2)]);

    // Not one row of it left the far side: the answer itself has none.
    let ran = super::world::raw(
        &link,
        &world.home,
        settings().deadline,
        &sessions(&["s-over", "s-small"]),
    );
    let text = String::from_utf8(ran.stdout).unwrap();
    let naming: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("s-over"))
        .collect();
    assert_eq!(naming, [r#"["skip","s-over","usage-rows"]"#]);
    assert!(
        text.ends_with("[\"end\",\"ok\",1,2,0,0]\n"),
        "{}",
        &text[text.len() - 40..]
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_missing_oversized_or_non_json_active_sessions_file_is_none() {
    let Some(world) = World::start() else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let file = format!("{}/runtime/active_sessions.json", world.home);
    // Missing: the folder, then the file.
    assert_eq!(reader.held(&link, &slot, &world.home), Ok(None));
    fs::create_dir_all(format!("{}/runtime", world.home)).unwrap();
    assert_eq!(reader.held(&link, &slot, &world.home), Ok(None));
    // Not JSON, and cut JSON.
    for text in ["not json at all", r#"[{"session_id":"s""#, ""] {
        fs::write(&file, text).unwrap();
        assert_eq!(reader.held(&link, &slot, &world.home), Ok(None), "{text}");
    }
    // Too long, though it is JSON.
    let long = format!("[\"{}\"]", "x".repeat(300 * 1024));
    fs::write(&file, &long).unwrap();
    assert_eq!(reader.held(&link, &slot, &world.home), Ok(None));
    // A link to a file is not followed.
    fs::write(format!("{}/elsewhere.json", world.home), "[]").unwrap();
    fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(format!("{}/elsewhere.json", world.home), &file).unwrap();
    assert_eq!(reader.held(&link, &slot, &world.home), Ok(None));
    // And one that is there, short and JSON comes back as it is.
    fs::remove_file(&file).unwrap();
    fs::write(&file, "[]").unwrap();
    assert_eq!(
        reader.held(&link, &slot, &world.home),
        Ok(Some("[]".into()))
    );
    let log = world.logged();
    assert!(log.iter().all(|line| !line.contains("failed")), "{log:?}");
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn no_far_command_line_is_longer_than_the_limit() {
    let Some(world) = World::start() else { return };
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    // The longest ids there can be, of the characters that grow most
    // when they are quoted for JSON and then for the far shell.
    let ids: Vec<String> = (0..60)
        .map(|number| format!("{number:02}{}", "'\\".repeat(far::LONGEST_ID / 2 - 1)))
        .collect();
    assert!(ids
        .iter()
        .all(|id| id.len() == far::LONGEST_ID && far::usable_id(id)));
    let reads = reader.reads(&world.home, ids.clone());
    let mut longest = 0;
    for read in &reads {
        let request = Request::Sessions { ids: read.clone() };
        let args = far::arguments(
            &world.home,
            settings().deadline,
            settings().most_output,
            &request,
        );
        longest = longest.max(super::super::batches::line_length(&args));
    }
    println!(
        "  {} reads for 60 ids, the longest line {longest} bytes",
        reads.len()
    );
    assert!(longest <= super::super::batches::LONGEST_LINE && longest > 2_000);
    assert_eq!(reads.iter().map(Vec::len).sum::<usize>(), 60);
    // And every one of them is sent: the link, which refuses a line over
    // 4096 bytes, refuses none.
    let began = Instant::now();
    let read = reader.sessions(&link, &slot, &world.home, &ids);
    println!("  all sent in {:?}", began.elapsed());
    assert_eq!(read.stopped, None);
    assert_eq!(read.gone, ids);
    assert!(world.logged().iter().all(|line| !line.contains("not run")));
    // An id that is not one is never sent at all.
    let odd = owned(&["", "a\nb", &"x".repeat(far::LONGEST_ID + 1)]);
    let read = reader.sessions(&link, &slot, &world.home, &odd);
    assert_eq!(read.skipped.len(), 3);
    assert!(read.skipped.iter().all(|(_, why)| *why == Skip::Id));
    assert_eq!(read.stopped, None::<Failure>);
    link.close();
}

/// The time and the bytes of each kind of read in the lab, five of each,
/// printed. The database has 400 sessions with three usage rows each.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn the_time_and_bytes_of_each_kind_of_read() {
    let Some(world) = World::start() else { return };
    let db = world.db();
    db.execute_batch("BEGIN").unwrap();
    let names: Vec<String> = (0..400)
        .map(|number| format!("20261113_{number:06}_abcdef"))
        .collect();
    for id in &names {
        world.add_session(&db, id, 3);
    }
    db.execute_batch("COMMIT").unwrap();
    fs::create_dir_all(format!("{}/runtime", world.home)).unwrap();
    let held = r#"[{"session_id":"20261113_000001_abcdef","surface":"desktop","pid":4242}]"#;
    fs::write(format!("{}/runtime/active_sessions.json", world.home), held).unwrap();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&machine());
    let nothing = crate::ssh_link::unix::tests::lab::far("true", &[]);
    let mut floor = Vec::new();
    for _ in 0..5 {
        let began = Instant::now();
        link.run(&nothing).unwrap();
        floor.push(began.elapsed().as_millis());
    }
    for _ in 0..5 {
        assert_eq!(
            reader.read(&link, &slot, &world.home, &Request::Probe),
            Ok(Reply::Probed)
        );
        let page = reader
            .prints(&link, &slot, &world.home, 100, 100, None)
            .unwrap();
        assert_eq!((page.prints.len(), page.ended), (100, false));
        // The five sessions that ended last.
        let since = reader
            .prints(&link, &slot, &world.home, 0, 500, Some(1_760_001_290.0))
            .unwrap();
        assert_eq!(
            (since.prints.len(), since.seen, since.ended),
            (5, 400, true)
        );
        let read = reader.sessions(&link, &slot, &world.home, &names[100..125]);
        assert_eq!((read.read.len(), read.stopped), (25, None));
        assert_eq!(
            reader.held(&link, &slot, &world.home).unwrap().as_deref(),
            Some(held)
        );
    }
    println!("  an empty command over the open link: {floor:?} ms");
    println!(
        "  the program is {} bytes on standard input",
        far::PROGRAM.len()
    );
    for kind in ["probe", "prints", "prints-since", "sessions", "held"] {
        let start = format!("history {kind} ok exit=0 ");
        let log = world.logged();
        let reads: Vec<(u32, u32)> = log
            .iter()
            .filter_map(|line| line.strip_prefix(&start))
            .filter_map(|rest| {
                let mut words = rest.split(' ');
                let ms = words.next()?.parse().ok()?;
                Some((ms, words.nth(1)?.parse().ok()?))
            })
            .collect();
        let ms: Vec<u32> = reads.iter().map(|read| read.0).collect();
        assert!(ms.len() >= 5, "{kind}: {log:?}");
        println!(
            "  {kind}: {} bytes, {:?} ms",
            reads[reads.len() - 1].1,
            &ms[ms.len() - 5..]
        );
    }
    link.close();
}
