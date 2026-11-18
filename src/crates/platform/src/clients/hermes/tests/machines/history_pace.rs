//! What ticket 61's verifier found the pace and the refused sessions
//! still lacked (ticket 63), in the lab of `history.rs`: the quick pace
//! ends once a turn has, a session that waits on a request of Hermes's
//! with no card here does not hold it, and the machine's status counts
//! the sessions the mirror would not take.
use super::{
    history::{plan, session, ticks_until, used, Site, OLD},
    history_leftovers::ticks,
    until,
};
use crate::clients::hermes::{history::Plan, machines::words};
use agent_hud_agents::hermes::gateway::Profile;
use rusqlite::Connection;
use serde_json::json;
use std::time::{Duration, Instant};

/// By the clock, a fifth of a second between ticks while a session
/// works and ten minutes otherwise, which no test waits out.
fn paced() -> Plan {
    Plan {
        clock: true,
        first: Duration::from_secs(1),
        live: Duration::from_millis(200),
        idle: Duration::from_secs(600),
        ..plan()
    }
}

fn three(db: &Connection) {
    for id in ["s-one", "s-two", "s-three"] {
        session(db, id, OLD);
        used(db, id, "model-a", "", 100);
    }
}

/// How many ticks come in `time`, from a moment later than now.
fn ticks_in(site: &Site, time: Duration) -> usize {
    let before = ticks(site);
    std::thread::sleep(time);
    ticks(site) - before
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_session_that_waits_on_a_request_with_no_card_does_not_hold_the_quick_pace() {
    // A session in a turn that waits on a request Hermes makes of its
    // viewers, of a kind Agent HUD never shows: Hermes lists it as
    // `working` (as it does a set of questions, in the recordings
    // `question-batch-*.json`), and no card says it waits.
    let lists = |gateway: &super::Fake| {
        let mut world = gateway.world();
        let stream = world.stream_in("default", "s-asking");
        stream.epoch = "epoch-1".into();
        stream.running = true;
        stream.open = vec![json!({"id": "srq-1", "method": "secret",
            "params": {"prompt": "A key, please"}})];
    };
    let Some(site) = Site::listing(paced(), three, lists) else {
        return;
    };
    until("the first whole sweep", || {
        site.history.ages().checked_ms.is_some()
    });
    until("it is listed", || {
        let first = site.fleet.writer("a").first(&Profile::Default, 0);
        first.contains(&"s-asking".to_owned())
    });
    // It waits, and writes nothing until it is answered: the slow pace.
    std::thread::sleep(Duration::from_millis(500));
    let waiting = ticks_in(&site, Duration::from_secs(3));
    assert_eq!(
        waiting, 0,
        "ticked at the quick pace while the one session in a turn waits on a request"
    );
    // Answered elsewhere, its turn goes on: the quick pace is back.
    let began = Instant::now();
    site.gateway
        .world()
        .stream_in("default", "s-asking")
        .open
        .clear();
    site.gateway.runs("default", "s-asking", true);
    let before = ticks(&site);
    until("five ticks at the quick pace", || {
        ticks(&site) >= before + 5
    });
    println!(
        "waiting on a request with no card: 0 ticks in 3 s; answered: 5 ticks in {} ms",
        began.elapsed().as_millis()
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_turn_that_ended_leaves_the_quick_pace_once_its_bound_has_passed() {
    // A turn keeps the quick pace three seconds past the last poll that
    // found it, where the app keeps it two minutes.
    let worked = Duration::from_secs(3);
    let plan = Plan { worked, ..paced() };
    let Some(site) = Site::with(plan, three) else {
        return;
    };
    until("the first whole sweep", || {
        site.history.ages().checked_ms.is_some()
    });
    // A session takes a turn: the ticks come at the quick pace.
    site.gateway.runs("default", "s-turn", true);
    let before = ticks(&site);
    until("five ticks at the quick pace", || {
        ticks(&site) >= before + 5
    });
    // The turn ends. Once the bound has passed, and a tick that was
    // under way then has made its first read (a busy machine is given
    // five seconds for that), no tick comes at the quick pace.
    site.gateway.runs("default", "s-turn", false);
    let grace = worked + Duration::from_secs(5);
    std::thread::sleep(grace);
    let after = ticks_in(&site, Duration::from_secs(3));
    assert_eq!(after, 0, "still at the quick pace after the turn ended");
    println!(
        "a turn ended: no tick from {} ms after its end, for 3 s",
        grace.as_millis()
    );
}

/// Twenty good sessions, and one whose two usage rows have a blob for
/// their model: two names in Hermes's table, one in the mirror.
fn with_one_refused(db: &Connection) {
    session(db, "s-bad", OLD + 8.5);
    for (blob, input) in [("x'01'", 40), ("x'02'", 50)] {
        let sql = format!(
            "INSERT INTO session_model_usage (session_id, model, input_tokens)
             VALUES ('s-bad', {blob}, {input})"
        );
        db.execute(&sql, []).unwrap();
    }
    for n in 0..20 {
        session(db, &format!("s-{n:02}"), OLD + f64::from(n));
        used(db, &format!("s-{n:02}"), "model-a", "", 100);
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_machines_status_counts_the_sessions_the_mirror_refused() {
    let Some(mut site) = Site::with(plan(), with_one_refused) else {
        return;
    };
    site.sync();
    // The status says one session is not counted, beside its two times.
    let said = words::set_aside(1);
    let mut row = serde_json::Value::Null;
    until("the status counts the session refused", || {
        row = site.fleet.stands("Mini", "connected");
        row["note"] == said.as_str()
    });
    assert!(row["historyCheckedAt"].is_string(), "{row}");
    assert_eq!(site.history.ages().refused, 1);
    // Once its far rows can be told apart it is kept, and the note goes.
    site.db
        .execute(
            "UPDATE session_model_usage SET model='model-' || input_tokens, input_tokens=input_tokens+1
             WHERE session_id='s-bad'",
            [],
        )
        .unwrap();
    ticks_until(&site, 3, || site.history.ages().refused == 0);
    until("the note goes", || {
        row = site.fleet.stands("Mini", "connected");
        row.get("note").is_none_or(serde_json::Value::is_null)
    });
    println!("one session refused: {said:?}; kept once its rows differ, and the note went");
}
