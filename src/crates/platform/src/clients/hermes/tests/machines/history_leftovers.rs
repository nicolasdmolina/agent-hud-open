//! What ticket 37's verifier found the refresh's tests did not hold
//! (ticket 61), in the lab of `history.rs`: the pace that goes by work
//! and not by what is listed, the sweep's place across a relaunch with a
//! page half written, the first queue's turn-taking, and a session the
//! mirror will not take.
use super::{
    history::{mirrored, now, plan, session, ticks_until, used, Site, OLD},
    until,
};
use crate::clients::hermes::{history::History, history::Plan, Pace};
use agent_hud_agents::hermes::gateway::Profile;
use rusqlite::Connection;
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// How many ticks the site's refresh has made: each begins with one
/// read of what the desktop app holds, once no sweep is overdue.
fn ticks(site: &Site) -> usize {
    let held = |read: &&String| read.starts_with("held ok ");
    site.reads().iter().filter(held).count()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn many_idle_listed_sessions_leave_the_slow_pace_and_a_working_one_brings_the_quick_one() {
    // By the clock. The quick pace is a fifth of a second and the slow
    // one ten minutes, which no test waits out: a tick that comes while
    // nothing works came at the quick pace, and is the fault looked for.
    let plan = Plan {
        clock: true,
        first: Duration::from_secs(1),
        live: Duration::from_millis(200),
        idle: Duration::from_secs(600),
        ..plan()
    };
    let seed = |db: &Connection| {
        for id in ["s-one", "s-two", "s-three"] {
            session(db, id, OLD);
            used(db, id, "model-a", "", 100);
        }
    };
    // Twenty-five sessions the gateway lists from its first poll on,
    // all idle: as a gateway that keeps its old chats listed.
    const LISTED: usize = 25;
    let lists = |gateway: &super::Fake| {
        let mut world = gateway.world();
        for n in 0..LISTED {
            let stream = world.stream_in("default", &format!("s-idle-{n:02}"));
            stream.epoch = "epoch-1".into();
            stream.row = json!({"title": "Idle", "model": "claude-test", "source": "telegram",
                "started_at": 1_760_000_000.0, "last_active": 1_760_000_100.0});
        }
    };
    let Some(mut site) = Site::listing(plan, seed, lists) else {
        return;
    };
    // The first sync, at the quick pace since a sweep that never
    // finished is overdue; and every listed session has its row.
    until("the first whole sweep", || {
        site.history.ages().checked_ms.is_some()
    });
    until("the idle sessions are listed", || {
        site.read().len() == 3 + LISTED
    });
    assert_eq!(
        site.fleet.writer("a").first(&Profile::Default, 0).len(),
        LISTED
    );

    // Listed and idle: the refresh is at its slow pace. Fifteen quick
    // paces pass and it does not tick. One tick may still have been
    // under way when the sweep ended.
    std::thread::sleep(Duration::from_millis(500));
    let before = ticks(&site);
    std::thread::sleep(Duration::from_secs(3));
    let idle = ticks(&site) - before;
    assert_eq!(idle, 0, "ticked at the quick pace with every session idle");

    // One of them takes a turn: the long wait ends, and the ticks come
    // at the quick pace for as long as it works.
    let began = Instant::now();
    site.gateway.runs("default", "s-idle-07", true);
    until("five ticks at the quick pace", || {
        ticks(&site) >= before + 5
    });
    let took = began.elapsed();
    println!(
        "{LISTED} idle listed: 0 ticks in 3 s; one working: 5 ticks in {} ms",
        took.as_millis()
    );
    // Nowhere near the slow pace's ten minutes.
    assert!(took < Duration::from_secs(120), "{took:?}");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_relaunch_with_a_page_half_written_reads_that_page_again() {
    // Four reads a tick, seven rows a page, two sessions a read: the
    // first tick reads a page of seven and writes six of them.
    let plan = Plan {
        reads: 4,
        page: 7,
        batch: 2,
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
    // In memory the sweep is past the page, with one session to read.
    assert_eq!(site.history.sweep(&Profile::Default), (7, 1));
    let count =
        |site: &Site| -> i64 { mirrored(&site.mirror(), "SELECT COUNT(*) FROM sessions")[0] };
    assert_eq!(count(&site), 6);
    // What the mirror keeps is where that page began: a refresh made
    // anew knows nothing of the session still to be read.
    let place: Vec<Option<i64>> = mirrored(&site.mirror(), "SELECT sweep_cursor FROM hud_reads");
    assert_eq!(place, [Some(0)]);

    // Client hooks off and on again, as a relaunch.
    site.fleet.stop();
    site.fleet.start_at(Pace {
        history: plan,
        ..super::pace()
    });
    assert_eq!(site.fleet.connected("Mini"), site.key);
    let history: Arc<History> = site.fleet.book().history_of("a").expect("its refresh");
    assert!(!Arc::ptr_eq(&history, &site.history));
    // It finishes its whole sweep: every session is in the mirror, the
    // seventh too, which a sweep that went on past its page would have
    // left for the next whole sweep, a day on.
    let mut ticked = history.tick();
    until("the whole sweep finishes", || {
        if ticked.reads == 0 || ticked.overdue {
            ticked = history.tick();
        }
        ticked.reads > 0 && !ticked.overdue
    });
    assert_eq!(count(&site), 30, "{:?}", site.reads());
    let seventh: Vec<i64> = mirrored(
        &site.mirror(),
        "SELECT COUNT(*) FROM hud_rows WHERE session_id='s-06' AND durable<>0",
    );
    assert_eq!(seventh, [1]);
    assert_eq!(site.read().len(), 30);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_first_queue_longer_than_a_tick_serves_every_session_in_turn() {
    // Four reads a tick, two sessions a read, four rows a page: the
    // first queue serves four sessions a tick, of twelve that are live.
    let plan = Plan {
        reads: 4,
        batch: 2,
        page: 4,
        ..plan()
    };
    let thirty = |db: &Connection| {
        for n in 0..30 {
            session(db, &format!("s-old-{n:02}"), OLD + f64::from(n));
            used(db, &format!("s-old-{n:02}"), "model-a", "", 100);
        }
    };
    let Some(site) = Site::with(plan, thirty) else {
        return;
    };
    const LIVE: usize = 12;
    for n in 0..LIVE {
        let id = format!("s-live-{n:02}");
        session(&site.db, &id, now() - 60.0);
        used(&site.db, &id, "model-a", "", 10);
        super::mirrors::works(&site.gateway, &id, "Live");
    }
    until("the polls list them", || {
        site.fleet.writer("a").first(&Profile::Default, 0).len() >= LIVE
    });
    site.sync();
    // The sweep has begun anew at the thirty old rows, a page of four a
    // tick: the three ticks below do not bring it to the live ones,
    // which are the last twelve rows.
    let (at, _) = site.history.sweep(&Profile::Default);
    assert!(at <= 12, "the sweep is at {at}");
    // Every live session's count grows.
    site.db
        .execute(
            "UPDATE session_model_usage SET input_tokens=5000 WHERE session_id LIKE 's-live-%'",
            [],
        )
        .unwrap();
    let grown = |site: &Site| -> Vec<String> {
        mirrored(
            &site.mirror(),
            "SELECT session_id FROM session_model_usage WHERE input_tokens=5000 ORDER BY 1",
        )
    };
    // Three ticks serve all twelve, four a tick, none twice before the
    // others had their turn: what a tick did not get to is first in the
    // next. A queue made anew each tick would serve the same four.
    let mut served = Vec::new();
    for tick in 1..=LIVE / 4 {
        site.history.tick();
        let now = grown(&site);
        assert_eq!(now.len(), tick * 4, "after tick {tick}: {now:?}");
        served.push(now.len());
    }
    let all: Vec<String> = (0..LIVE).map(|n| format!("s-live-{n:02}")).collect();
    assert_eq!(grown(&site), all);
    println!("the first queue served {served:?} of {LIVE} after each tick");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_session_the_mirror_refuses_is_set_aside_and_the_history_goes_on() {
    // Twenty good sessions, and among them one with two usage rows whose
    // model is a blob: two names in Hermes's table, one in the mirror.
    let seed = |db: &Connection| {
        for n in 0..20 {
            if n == 8 {
                session(db, "s-bad", OLD + 8.5);
                for (blob, input) in [("x'01'", 40), ("x'02'", 50)] {
                    let sql = format!(
                        "INSERT INTO session_model_usage (session_id, model, input_tokens)
                         VALUES ('s-bad', {blob}, {input})"
                    );
                    db.execute(&sql, []).unwrap();
                }
            }
            session(db, &format!("s-{n:02}"), OLD + f64::from(n));
            used(db, &format!("s-{n:02}"), "model-a", "", 100);
        }
    };
    let Some(mut site) = Site::with(plan(), seed) else {
        return;
    };
    // The first whole sweep finishes all the same, in a tick or two.
    let mut ticks = 0;
    let synced = (1..=4).any(|_| {
        ticks += 1;
        let ticked = site.history.tick();
        ticked.reads > 0 && !ticked.overdue
    });
    let log = site.fleet.log();
    assert!(synced, "the sweep never finished:\n{log}");
    assert!(site.history.ages().checked_ms.is_some());
    assert!(!log.contains("history stopped"), "{log}");
    // Every other session of its batch is in the mirror and listed; the
    // one refused is not, and the log names it once.
    let ids: Vec<String> = mirrored(&site.mirror(), "SELECT id FROM sessions ORDER BY 1");
    assert_eq!(ids.len(), 20, "{ids:?}");
    assert!(!ids.contains(&"s-bad".to_owned()));
    assert_eq!(site.read().len(), 20);
    let refused = |site: &Site| -> usize {
        let line = "mirror refused profile=default session=s-bad why=UNIQUE";
        (site.fleet.log().lines())
            .filter(|logged| logged.contains(line))
            .count()
    };
    assert_eq!(refused(&site), 1, "{log}");

    // Set aside as it is: whole sweeps go on over it and it is not
    // asked for again, nor refused again.
    let asked = |site: &Site| {
        let sessions = |read: &&String| read.starts_with("sessions ok ");
        site.reads().iter().filter(sessions).count()
    };
    let before = asked(&site);
    for _ in 0..3 {
        let ticked = site.history.tick();
        assert!(ticked.reads > 0 && !ticked.overdue);
    }
    assert_eq!((asked(&site), refused(&site)), (before, 1));
    // A session that ends later is read as ever.
    session(&site.db, "s-later", now() - 30.0);
    used(&site.db, "s-later", "model-a", "", 50);
    ticks_until(&site, 3, || {
        mirrored::<i64>(&site.mirror(), "SELECT COUNT(*) FROM sessions")[0] == 21
    });
    assert_eq!(site.read().len(), 21);

    // Once its far rows can be told apart, it is read and kept too.
    site.db
        .execute(
            "UPDATE session_model_usage SET model='model-' || input_tokens, input_tokens=input_tokens+1
             WHERE session_id='s-bad'",
            [],
        )
        .unwrap();
    let kept = |site: &Site| -> Vec<i64> {
        mirrored(
            &site.mirror(),
            "SELECT input_tokens FROM session_model_usage WHERE session_id='s-bad' ORDER BY 1",
        )
    };
    ticks_until(&site, 3, || kept(&site) == [41, 51]);
    assert_eq!(site.read().len(), 22);
    assert_eq!(refused(&site), 1);
    println!("synced in {ticks} ticks with one session refused; kept once its rows differ");
}
