//! The request and the answer, with no process run. The program itself
//! runs in the platform crate's lab (`clients/hermes/far_history`).
use super::*;
use crate::hermes::gateway::machine_key;
use std::time::Duration;

fn answer(lines: &[&str]) -> Vec<u8> {
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

fn ids(names: &[&str]) -> Request {
    Request::Sessions {
        ids: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

const STARTED: &str = r#"["started",4242]"#;
const S1: &str = r#"["s",3,"s1",1760000001.0,"T","/w","cli","m","p","https://x","api",10,2,3,4,null,1760000601.5,null,"/1760000601.5/10/2/3/4/1/1.0/2.0/3.0/4.0/9.0"]"#;
const U1: &str =
    r#"["u","s1","m0","p","https://x","api","aux",1,2,3,4,1760000010.25,1760000600.0]"#;

/// Every double-quoted piece of the program's text, in order.
fn quoted(program: &str) -> Vec<&str> {
    program.split('"').skip(1).step_by(2).collect()
}

#[test]
fn the_program_is_one_short_file_of_the_standard_library_that_only_reads() {
    assert!(
        PROGRAM.lines().count() <= 300,
        "{}",
        PROGRAM.lines().count()
    );
    // Every import is of the standard library, and none is Hermes's.
    let allowed = [
        "hashlib",
        "json",
        "math",
        "os",
        "resource",
        "signal",
        "sqlite3",
        "stat",
        "subprocess",
        "sys",
        "threading",
        "time",
        "urllib.parse",
    ];
    let mut imported = Vec::new();
    for line in PROGRAM.lines().map(str::trim) {
        assert!(!line.starts_with("from "), "{line}");
        assert!(!line.contains("__import__") && !line.contains("importlib"));
        if let Some(module) = line.strip_prefix("import ") {
            assert!(
                allowed.contains(&module),
                "not the standard library: {module}"
            );
            imported.push(module);
        }
    }
    assert_eq!(imported.len(), allowed.len(), "{imported:?}");

    // No `immutable` anywhere, and the one connection cannot write.
    assert!(!PROGRAM.to_lowercase().contains("immutable"));
    assert_eq!(PROGRAM.matches("sqlite3.connect(").count(), 1);
    assert!(PROGRAM
        .contains(r#"sqlite3.connect("file:" + urllib.parse.quote(path) + "?mode=ro", uri=True"#));
    assert_eq!(PROGRAM.matches("mode=").count(), 1);

    // No statement but SELECT: no other SQL verb is anywhere in the text,
    // and each piece of SQL starts a SELECT or goes on inside one.
    for verb in [
        "INSERT",
        "UPDATE",
        "DELETE",
        "REPLACE",
        "CREATE",
        "DROP",
        "ALTER",
        "ATTACH",
        "DETACH",
        "PRAGMA",
        "VACUUM",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT",
        "REINDEX",
        "ANALYZE",
    ] {
        assert!(!PROGRAM.contains(verb), "{verb}");
    }
    assert!(!PROGRAM.contains("executescript") && !PROGRAM.contains("executemany"));
    assert_eq!(PROGRAM.matches(".execute(").count(), 3);
    let sql: Vec<&str> = quoted(PROGRAM)
        .into_iter()
        .filter(|text| text.contains(" FROM "))
        .collect();
    assert_eq!(sql.len(), 7, "{sql:?}");
    for text in sql {
        let starts = ["SELECT ", "(SELECT ", "+(SELECT ", " UNION ALL SELECT "];
        assert!(starts.iter().any(|start| text.starts_with(start)), "{text}");
    }

    // Nothing is opened for writing, and nothing is removed, renamed or
    // made. It writes to its own standard output and standard error.
    for word in [
        "O_WRONLY", "O_RDWR", "O_CREAT", "O_APPEND", "O_TRUNC", "unlink", "remove(", "rename",
        "mkdir", "makedirs", "shutil", "tempfile", "truncate", "chmod", "chown", "symlink",
    ] {
        assert!(!PROGRAM.contains(word), "{word}");
    }
    assert_eq!(PROGRAM.matches("open(").count(), 1);
    assert!(PROGRAM.contains("os.open(path, os.O_RDONLY | os.O_NONBLOCK"));
    assert_eq!(PROGRAM.matches("write(").count(), 2);
    assert!(PROGRAM.contains("os.write(1, view)") && PROGRAM.contains("os.write(2, ("));
}

#[test]
fn a_request_is_four_short_arguments() {
    let home = "/Users/far/.hermes";
    let machine = machine_key(Some("0123456789abcdef0123456789abcdef"), home).unwrap();
    let other = machine_key(Some("fedcba9876543210fedcba9876543210"), home).unwrap();
    let mark = mark(&machine, home);
    // The machine's key, a colon, and sixteen hex characters for the home.
    let (key, named) = mark[MARK.len()..].split_once(':').unwrap();
    assert_eq!(key, machine.as_str());
    assert_eq!(named.len(), 16);
    assert!(named
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    // Two homes of one machine share the start of the mark, by which a
    // reader of one finds a reader of the other. Two machines do not.
    let profile = super::mark(&machine, "/Users/far/.hermes/profiles/reports");
    assert_ne!(mark, profile);
    assert_eq!(profile.split_once(named), None);
    assert!(profile.starts_with(&format!("{MARK}{key}:")));
    assert!(!super::mark(&other, home).starts_with(&format!("{MARK}{key}:")));
    assert!(super::mark(&other, home).ends_with(&format!(":{named}")));

    let request = Request::Prints {
        after: 7,
        limit: 100,
        since: Some(1_760_000_000.5),
    };
    let [first, deadline, at, asked] =
        arguments(&machine, home, Duration::from_secs(20), 4096, &request);
    assert_eq!(
        (first, deadline.as_str(), at.as_str()),
        (mark, "20.000", home)
    );
    let asked: serde_json::Value = serde_json::from_str(&asked).unwrap();
    assert_eq!(
        asked,
        serde_json::json!({"k": "prints", "after": 7, "n": 100, "since": 1_760_000_000.5, "most": 4096})
    );
    let [.., asked] = arguments(
        &machine,
        home,
        Duration::from_millis(2500),
        9,
        &ids(&["a", "b\"c"]),
    );
    assert_eq!(asked, r#"{"ids":["a","b\"c"],"k":"sessions","most":9}"#);
    assert_eq!(Request::Probe.kind(), "probe");
    assert_eq!(request.kind(), "prints-since");

    assert!(usable_id("20261113_164214_abcdef"));
    for id in ["", "a\nb", "a\0b", &"x".repeat(LONGEST_ID + 1)] {
        assert!(!usable_id(id));
    }
}

#[test]
fn a_whole_answer_of_each_kind_is_read() {
    assert_eq!(
        parse(
            &Request::Probe,
            &answer(&[STARTED, r#"["end","ok",0,0,0,0]"#])
        ),
        Ok(Reply::Probed)
    );
    assert_eq!(
        parse(&Request::Held, &answer(&[r#"["no","busy"]"#])),
        Ok(Reply::No(No::Busy))
    );
    assert_eq!(
        parse(&Request::Probe, &answer(&[r#"["no","sqlite"]"#])),
        Ok(Reply::No(No::Sqlite))
    );

    let read = parse(
        &ids(&["s1", "s2", "big", "gone"]),
        &answer(&[
            STARTED,
            S1,
            U1,
            r#"["skip","big","usage-rows"]"#,
            &S1.replace("\"s1\"", "\"s2\"")
                .replace("[\"s\",3", "[\"s\",9"),
            r#"["end","ok",2,1,0,0]"#,
        ]),
    );
    let Ok(Reply::Sessions(batch)) = read else {
        panic!("{read:?}");
    };
    assert_eq!(batch.skipped, ["big"]);
    assert_eq!(batch.sessions.len(), 2);
    let first = &batch.sessions[0];
    assert_eq!((first.rowid, first.id.as_str()), (3, "s1"));
    assert_eq!(first.print, "/1760000601.5/10/2/3/4/1/1.0/2.0/3.0/4.0/9.0");
    assert_eq!(first.cells.len(), SESSION_COLUMNS.len());
    assert_eq!(first.cells[0], Cell::Real(1_760_000_001.0));
    assert_eq!(first.cells[1], Cell::Text("T".into()));
    assert_eq!(first.cells[8], Cell::Int(10));
    assert_eq!(first.cells[12], Cell::Null);
    assert_eq!(first.usage.len(), 1);
    assert_eq!(first.usage[0].len(), USAGE_COLUMNS.len());
    // The task and the base URL are each usage row's own.
    assert_eq!(first.usage[0][4], Cell::Text("aux".into()));
    assert_eq!(first.usage[0][2], Cell::Text("https://x".into()));
    assert_eq!(batch.sessions[1].usage.len(), 0);
    assert_eq!(
        rusqlite::types::Value::from(&first.cells[8]),
        rusqlite::types::Value::Integer(10)
    );

    let page = Request::Prints {
        after: 4,
        limit: 3,
        since: None,
    };
    let read = parse(
        &page,
        &answer(&[
            STARTED,
            r#"["p",5,"a","x/1"]"#,
            r#"["p",9,"b","x/2"]"#,
            r#"["end","ok",2,0,3,11]"#,
        ]),
    );
    let Ok(Reply::Prints(prints)) = read else {
        panic!("{read:?}");
    };
    assert_eq!(prints.prints.len(), 2);
    assert_eq!((prints.seen, prints.last, prints.ended), (3, 11, false));
    assert_eq!(
        prints.prints[1],
        Print {
            rowid: 9,
            id: "b".into(),
            print: "x/2".into()
        }
    );
    // Fewer rows than asked for: the table ended.
    let read = parse(&page, &answer(&[STARTED, r#"["end","ok",0,0,2,6]"#]));
    assert!(matches!(
        read,
        Ok(Reply::Prints(Prints {
            ended: true,
            seen: 2,
            last: 6,
            ..
        }))
    ));

    let held = answer(&[
        STARTED,
        r#"["held","[{\"session_id\":\"s\"}]"]"#,
        r#"["end","ok",0,0,0,0]"#,
    ]);
    assert_eq!(
        parse(&Request::Held, &held),
        Ok(Reply::Held(Some(r#"[{"session_id":"s"}]"#.into())))
    );
    let none = answer(&[STARTED, r#"["end","none",0,0,0,0]"#]);
    assert_eq!(parse(&Request::Held, &none), Ok(Reply::Held(None)));

    let end = |status: &str| answer(&[STARTED, &format!(r#"["end","{status}",0,0,0,0]"#)]);
    assert_eq!(parse(&ids(&["s1"]), &end("too-large")), Ok(Reply::TooLarge));
    assert_eq!(
        parse(&page, &end("no-database")),
        Ok(Reply::Failed(Far::NoDatabase))
    );
    assert_eq!(
        parse(&page, &end("db-locked")),
        Ok(Reply::Failed(Far::Locked))
    );
    assert_eq!(
        parse(&ids(&["s1"]), &end("db-schema")),
        Ok(Reply::Failed(Far::Schema))
    );
}

#[test]
fn an_answer_with_its_last_line_cut_or_counts_that_do_not_match_is_rejected_whole() {
    let asked = ids(&["s1"]);
    let whole = answer(&[STARTED, S1, U1, r#"["end","ok",1,1,0,0]"#]);
    assert!(matches!(parse(&asked, &whole), Ok(Reply::Sessions(_))));
    // Cut anywhere before its end, it is never read: not one row is used.
    for end in 0..whole.len() {
        assert!(parse(&asked, &whole[..end]).is_err(), "cut at {end}");
    }
    let no_end = answer(&[STARTED, S1, U1]);
    assert_eq!(parse(&asked, &no_end), Err(Rejected::NoCompletion));
    assert_eq!(
        parse(&asked, &answer(&[STARTED])),
        Err(Rejected::NoCompletion)
    );
    assert_eq!(parse(&asked, b""), Err(Rejected::NotStarted));
    assert_eq!(
        parse(&asked, &answer(&[S1, r#"["end","ok",1,0,0,0]"#])),
        Err(Rejected::NotStarted)
    );

    for end in [
        r#"["end","ok",2,1,0,0]"#,
        r#"["end","ok",1,0,0,0]"#,
        r#"["end","ok",0,1,0,0]"#,
        r#"["end","ok",1,2,0,0]"#,
    ] {
        assert_eq!(
            parse(&asked, &answer(&[STARTED, S1, U1, end])),
            Err(Rejected::Counts),
            "{end}"
        );
    }
    for end in [
        r#"["end","ok",1,1,0]"#,
        r#"["end","ok","1",1,0,0]"#,
        r#"["end",7,1,1,0,0]"#,
        r#"{"end":"ok"}"#,
        r#"["end","ok",1,1,0,0"#,
    ] {
        assert_eq!(
            parse(&asked, &answer(&[STARTED, S1, U1, end])),
            Err(Rejected::NoCompletion),
            "{end}"
        );
    }
    assert_eq!(
        parse(
            &asked,
            &answer(&[STARTED, S1, U1, r#"["end","fine",0,0,0,0]"#])
        ),
        Err(Rejected::Line(2))
    );
    assert_eq!(
        parse(&asked, &answer(&[STARTED, r#"["end","fine",0,0,0,0]"#])),
        Err(Rejected::Status)
    );
    assert_eq!(
        parse(&asked, &answer(&[STARTED, r#"["end","none",0,0,0,0]"#])),
        Err(Rejected::Status)
    );
    assert_eq!(
        parse(
            &asked,
            &answer(&[STARTED, r#"["end","too-large",1,0,0,0]"#])
        ),
        Err(Rejected::Counts)
    );
    assert_eq!(
        parse(
            &Request::Probe,
            &answer(&[STARTED, r#"["end","too-large",0,0,0,0]"#])
        ),
        Err(Rejected::Status)
    );

    let page = Request::Prints {
        after: 4,
        limit: 2,
        since: None,
    };
    let print = r#"["p",5,"a","x"]"#;
    for end in [
        r#"["end","ok",2,0,1,5]"#,
        r#"["end","ok",1,0,0,5]"#,
        r#"["end","ok",1,0,3,5]"#,
        r#"["end","ok",1,0,1,4]"#,
        r#"["end","ok",1,1,1,5]"#,
    ] {
        assert_eq!(
            parse(&page, &answer(&[STARTED, print, end])),
            Err(Rejected::Counts),
            "{end}"
        );
    }
}

#[test]
fn a_row_of_the_wrong_shape_rejects_the_whole_answer() {
    let asked = ids(&["s1", "s2"]);
    let end = r#"["end","ok",1,1,0,0]"#;
    let other = U1.replace("\"s1\"", "\"s2\"");
    let wrong = [
        // A session row with a cell too few, a cell too many, a cell that
        // is no single value, and an id that is not text.
        S1.replacen(",null,", ",", 1),
        S1.replacen(",null,", ",null,null,", 1),
        S1.replacen(",null,", ",[1],", 1),
        S1.replacen(",null,", ",true,", 1),
        S1.replacen(",null,", ",{\"a\":1},", 1),
        S1.replacen("\"s1\"", "17", 1),
        S1.replacen("[\"s\",3", "[\"s\",\"3\"", 1),
        // A session nobody asked for, and a line of no known kind.
        S1.replacen("\"s1\"", "\"s9\"", 1),
        S1.replacen("[\"s\"", "[\"x\"", 1),
        "{}".to_owned(),
        "17".to_owned(),
        "not json".to_owned(),
    ];
    for line in &wrong {
        assert_eq!(
            parse(&asked, &answer(&[STARTED, line, end])),
            Err(Rejected::Line(2)),
            "{line}"
        );
    }
    let third = [
        // A usage row of another session, one with a cell too few, and
        // the same session twice.
        other.clone(),
        U1.replacen(",1,2,", ",1,", 1),
        S1.to_owned(),
    ];
    for line in &third {
        assert_eq!(
            parse(&asked, &answer(&[STARTED, S1, line, end])),
            Err(Rejected::Line(3)),
            "{line}"
        );
    }
    // A usage row with no session before it, or after a skipped one.
    assert_eq!(
        parse(&asked, &answer(&[STARTED, U1, end])),
        Err(Rejected::Line(2))
    );
    let skip = r#"["skip","s2","usage-rows"]"#;
    assert_eq!(
        parse(&asked, &answer(&[STARTED, S1, skip, U1, end])),
        Err(Rejected::Line(4))
    );
    // A session named both as read and as skipped, and a skip with a
    // reason nobody knows.
    let twice = r#"["skip","s1","usage-rows"]"#;
    assert_eq!(
        parse(&asked, &answer(&[STARTED, S1, twice, end])),
        Err(Rejected::Line(3))
    );
    let why = r#"["skip","s2","because"]"#;
    assert_eq!(
        parse(&asked, &answer(&[STARTED, why, end])),
        Err(Rejected::Line(2))
    );

    // More usage rows than the reader accepts: the 501st rejects it all.
    let mut lines = vec![STARTED.to_owned(), S1.to_owned()];
    lines.extend((0..=MOST_USAGE_ROWS).map(|_| U1.to_owned()));
    lines.push(r#"["end","ok",1,501,0,0]"#.to_owned());
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse(&asked, &answer(&lines)),
        Err(Rejected::Line(MOST_USAGE_ROWS + 3))
    );

    // Fingerprints: out of order, before the page, or of the wrong shape.
    let page = Request::Prints {
        after: 4,
        limit: 5,
        since: None,
    };
    let end = r#"["end","ok",2,0,2,9]"#;
    for (first, second) in [
        (r#"["p",6,"a","x"]"#, r#"["p",6,"b","y"]"#),
        (r#"["p",6,"a","x"]"#, r#"["p",5,"b","y"]"#),
        (r#"["p",6,"a","x"]"#, r#"["p",7,"","y"]"#),
        (r#"["p",6,"a","x"]"#, r#"["p",7,"b",9]"#),
        (r#"["p",6,"a","x"]"#, S1),
    ] {
        assert_eq!(
            parse(&page, &answer(&[STARTED, first, second, end])),
            Err(Rejected::Line(3)),
            "{second}"
        );
    }
    assert_eq!(
        parse(&page, &answer(&[STARTED, r#"["p",4,"a","x"]"#, end])),
        Err(Rejected::Line(2))
    );

    // A refusal says one thing and nothing after it.
    assert_eq!(
        parse(&asked, &answer(&[r#"["no","busy"]"#, S1])),
        Err(Rejected::NotStarted)
    );
    assert_eq!(
        parse(&asked, &answer(&[r#"["no","whatever"]"#])),
        Err(Rejected::Line(1))
    );
    // What a rejection says never holds what the row held.
    assert_eq!(Rejected::Line(3).to_string(), "line 3 has the wrong shape");
}

#[test]
fn the_first_line_says_whether_the_reader_started() {
    assert_eq!(first_line(b""), First::Pending);
    assert_eq!(first_line(br#"["started",42"#), First::Pending);
    assert_eq!(first_line(b"[\"started\",4242]\n"), First::Started);
    assert_eq!(first_line(b"[\"started\",4242]\n[\"end\""), First::Started);
    assert_eq!(first_line(b"[\"no\",\"busy\"]\n"), First::Other);
    assert_eq!(first_line(b"[\"started\"]\n"), First::Other);
    assert_eq!(first_line(b"Last login: today\n"), First::Other);
    assert_eq!(first_line(&[b'x'; 300]), First::Other);
}

/// The fingerprint is two stamps and then the counts exactly as this
/// crate's reader takes them, so a far print and a local one of the same
/// row are one text.
#[test]
fn a_fingerprint_is_the_stamps_and_the_readers_own_counts() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, started_at REAL NOT NULL, ended_at REAL, input_tokens INTEGER, output_tokens INTEGER,
            cache_read_tokens INTEGER, cache_write_tokens INTEGER, last_activity_at REAL);
         CREATE TABLE session_model_usage (session_id TEXT NOT NULL, model TEXT NOT NULL, input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0, cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0, last_seen REAL);
         INSERT INTO sessions VALUES ('a', 1, NULL, 10, 2, 3, 4, 1760000600.5), ('b', 2, 1760000700, NULL, 0, 0, 0, NULL);
         INSERT INTO session_model_usage VALUES ('a', 'm', 1, 2, 3, 4, 9.5), ('a', 'n', 10, 20, 30, 40, 8);",
    )
    .unwrap();
    let s = sql_columns(&db, "sessions").unwrap();
    let u = sql_columns(&db, "session_model_usage").unwrap();
    let counts = session_counts::sql(&s, &u);
    let own = |id: &str| -> String {
        db.query_row(
            &format!("SELECT {counts} FROM sessions s WHERE s.id=?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
    };
    let prints = local_prints(&db, 0, 10).unwrap();
    assert_eq!(prints.len(), 2);
    assert_eq!(prints[0].print, format!("/1760000600.5/{}", own("a")));
    assert_eq!(
        prints[0].print,
        "/1760000600.5/10/2/3/4/2/11.0/22.0/33.0/44.0/9.5"
    );
    assert_eq!(prints[1].print, format!("1760000700.0//{}", own("b")));
    assert_eq!(local_prints(&db, 1, 10).unwrap().len(), 1);
}

/// The verifier's probe of the first review of this reader
/// (`evidence/36/verifier-report.md`, finding 3): the last line's counts
/// were not looked at for a probe or the held file, and a sessions read
/// took any page numbers. Every number of the last line now says what
/// came, for every kind of answer.
#[test]
fn v_completion_counts_are_checked_for_every_kind() {
    let start = "[\"started\",42]\n";
    let cases = [
        (
            "probe",
            Request::Probe,
            "[\"end\",\"ok\",7,9,0,0]\n".to_owned(),
        ),
        (
            "held",
            Request::Held,
            "[\"held\",\"{}\"]\n[\"end\",\"ok\",7,9,0,0]\n".to_owned(),
        ),
        (
            "sessions",
            ids(&[]),
            "[\"end\",\"ok\",0,0,-1,-9]\n".to_owned(),
        ),
    ];
    let mut accepted = Vec::new();
    for (kind, ask, rest) in cases {
        let answer = format!("{start}{rest}");
        let got = parse(&ask, answer.as_bytes());
        println!("{kind} malformed counts -> {got:?}");
        if got.is_ok() {
            accepted.push(kind);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted mismatched completion counts: {accepted:?}"
    );

    // Each number, one at a time: only the true ones pass.
    let held = r#"["held","{}"]"#;
    let kinds: [(Request, Vec<&str>, [i64; 4]); 3] = [
        (Request::Probe, vec![], [0, 0, 0, 0]),
        (Request::Held, vec![held], [0, 0, 0, 0]),
        (ids(&["s1"]), vec![S1, U1], [1, 1, 0, 0]),
    ];
    let said = |rows: &[&str], counts: [i64; 4]| {
        let [a, b, c, d] = counts;
        let end = format!(r#"["end","ok",{a},{b},{c},{d}]"#);
        let mut lines = vec![STARTED];
        lines.extend(rows);
        lines.push(&end);
        answer(&lines)
    };
    for (ask, rows, right) in kinds {
        assert!(
            parse(&ask, &said(&rows, right)).is_ok(),
            "{ask:?} {right:?}"
        );
        for at in 0..4 {
            for wrong in [-1, right[at] + 1, 9_000] {
                let mut counts = right;
                counts[at] = wrong;
                assert_eq!(
                    parse(&ask, &said(&rows, counts)),
                    Err(Rejected::Counts),
                    "{ask:?} {counts:?}"
                );
            }
        }
    }
    // A page of fingerprints: after rowid 4, at most 5 rows. Its numbers
    // are the fingerprints, nothing, the rows looked at and the last
    // one's rowid, which is past `after` by at least one for each row.
    let page = Request::Prints {
        after: 4,
        limit: 5,
        since: None,
    };
    let print = r#"["p",6,"a","x"]"#;
    for right in [[1, 0, 2, 6], [1, 0, 2, 7], [1, 0, 3, 7], [1, 0, 5, 900]] {
        assert!(parse(&page, &said(&[print], right)).is_ok(), "{right:?}");
    }
    for wrong in [
        [0, 0, 2, 7],
        [2, 0, 2, 7],
        [-1, 0, 2, 7],
        [1, 1, 2, 7],
        [1, -1, 2, 7],
        [1, 0, 0, 7],
        [1, 0, -1, 7],
        [1, 0, 6, 70],
        [1, 0, 4, 7],
        [1, 0, 2, 5],
        [1, 0, 2, -1],
    ] {
        assert_eq!(
            parse(&page, &said(&[print], wrong)),
            Err(Rejected::Counts),
            "{wrong:?}"
        );
    }
    assert!(parse(&page, &said(&[], [0, 0, 0, 4])).is_ok());
    assert!(parse(&page, &said(&[], [0, 0, 3, 9])).is_ok());
    for wrong in [
        [0, 0, 0, 5],
        [0, 0, 0, 3],
        [0, 0, 1, 4],
        [0, 0, -1, 4],
        [0, 0, 0, -1],
    ] {
        assert_eq!(
            parse(&page, &said(&[], wrong)),
            Err(Rejected::Counts),
            "{wrong:?}"
        );
    }
    // A status that brings no rows brings no numbers either.
    let none = answer(&[STARTED, r#"["end","none",0,0,0,1]"#]);
    assert_eq!(parse(&Request::Held, &none), Err(Rejected::Counts));
}
