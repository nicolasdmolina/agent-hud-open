//! A far session its gateway lists as working for days (ticket 67), in
//! the lab of `history.rs`: once nothing has changed in any listed
//! session for the quiet time, the gateway is polled at the model's
//! slow pace and the machine's history is read at its slow one; a
//! stream that moves brings both quick paces back; and a request asked
//! in the quiet session shows its card within the bound for a gateway
//! where nothing is busy.
//!
//! The quiet time here is four seconds, where the app's is thirty
//! minutes. The model's own waits are the app's: a second, and ten.
use super::{
    history::{plan, session, used, Site, OLD},
    history_leftovers::ticks,
    until,
};
use crate::clients::hermes::{history::Plan, Pace};
use agent_hud_agents::hermes::gateway::{Profile, IDLE_POLL_MS};
use rusqlite::Connection;
use serde_json::json;
use std::time::{Duration, Instant};

pub(super) const QUIET: Duration = Duration::from_secs(4);
/// How long a turn a poll found keeps the history's quick pace here.
pub(super) const WORKED: Duration = Duration::from_secs(2);
/// The bound of spec section 10 for a card where every listed session
/// was idle or none was listed.
pub(super) const IDLE_CARD: Duration = Duration::from_secs(13);
pub(super) const STUCK: &str = "s-stuck";

/// By the clock: a fifth of a second between two history ticks while a
/// session works, and ten minutes otherwise, which no test waits out.
pub(super) fn paced() -> Pace {
    let history = Plan {
        clock: true,
        first: Duration::from_secs(1),
        live: Duration::from_millis(200),
        idle: Duration::from_secs(600),
        worked: WORKED,
        ..plan()
    };
    Pace {
        quiet: QUIET,
        history,
        // Every poll is said in the log, which is how they are counted.
        trace: true,
        ..super::pace()
    }
}

pub(super) fn three(db: &Connection) {
    for id in ["s-one", "s-two", "s-three"] {
        session(db, id, OLD);
        used(db, id, "model-a", "", 100);
    }
}

/// What the Mac Mini's default gateway listed on 2026-11-18: one
/// session as working, whose stream never moves, and its old chats,
/// idle. None of them ever says a word.
pub(super) fn as_the_mini_lists(gateway: &super::Fake) {
    let mut world = gateway.world();
    for n in 0..10 {
        world.stream_in("default", &format!("s-old-{n}")).epoch = "epoch-1".into();
    }
    let stream = world.stream_in("default", STUCK);
    stream.epoch = "epoch-1".into();
    stream.running = true;
}

/// How many polls of the gateway ran to their end, by the log.
pub(super) fn polls(site: &Site) -> usize {
    let log = site.fleet.log();
    log.lines()
        .filter(|line| line.contains(" trace poll ms="))
        .count()
}

/// The polls and the history ticks that come in `time`.
fn in_the_next(site: &Site, time: Duration) -> (usize, usize) {
    let before = (polls(site), ticks(site));
    std::thread::sleep(time);
    (polls(site) - before.0, ticks(site) - before.1)
}

/// The site once its first sync is done, the stuck session is listed,
/// and the quiet time and the turn's own bound have passed, with five
/// seconds more for a tick that was under way.
fn quiet_site() -> Option<(Site, Instant)> {
    let site = Site::paced(paced(), three, as_the_mini_lists)?;
    until("it is listed", || {
        let first = site.fleet.writer("a").first(&Profile::Default, 0);
        first.contains(&STUCK.to_owned())
    });
    let listed = Instant::now();
    until("the first whole sweep", || {
        site.history.ages().checked_ms.is_some()
    });
    let settled = listed + QUIET + WORKED + Duration::from_secs(5);
    std::thread::sleep(settled.saturating_duration_since(Instant::now()));
    Some((site, listed))
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_session_listed_as_working_for_ever_leaves_both_quick_paces_once_it_is_quiet() {
    let Some((site, _)) = quiet_site() else {
        return;
    };
    // Five seconds: at the close watch that is some 160 polls, and at
    // the history's quick pace some 25 ticks. At the model's slow pace
    // it is one poll at most, and no tick.
    let window = Duration::from_secs(5);
    let (polled, ticked) = in_the_next(&site, window);
    assert!(
        polled <= 1,
        "{polled} polls in 5 s with nothing moving: not the slow pace of {IDLE_POLL_MS} ms"
    );
    assert_eq!(
        ticked, 0,
        "history ticks at the quick pace: {ticked} in 5 s"
    );
    // Hermes still lists it as working, and the HUD still says so.
    let row = |site: &Site| {
        let first = site.fleet.writer("a").first(&Profile::Default, 0);
        first.contains(&STUCK.to_owned())
    };
    assert!(row(&site), "it is still listed");
    // Its stream moves, with no hint from the gateway: the next poll
    // finds it, and both quick paces are back.
    let moved = Instant::now();
    site.gateway
        .tells_quietly("default", STUCK, "tool.progress", json!({}));
    let before = (polls(&site), ticks(&site));
    until("the polls are quick again", || {
        polls(&site) >= before.0 + 20
    });
    let quick = moved.elapsed();
    until("five ticks at the quick pace", || {
        ticks(&site) >= before.1 + 5
    });
    assert!(
        quick <= IDLE_CARD,
        "its move took {} ms to count",
        quick.as_millis()
    );
    println!(
        "quiet: {polled} polls and {ticked} ticks in 5 s; moved: 20 polls within {} ms, 5 ticks within {} ms",
        quick.as_millis(),
        moved.elapsed().as_millis()
    );
    // It stands still again, and is quiet again a quiet time later.
    std::thread::sleep(QUIET + WORKED + Duration::from_secs(5));
    let (polled, ticked) = in_the_next(&site, window);
    assert!(polled <= 1, "{polled} polls in 5 s, quiet a second time");
    assert_eq!(ticked, 0, "{ticked} ticks in 5 s, quiet a second time");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_request_asked_in_a_quiet_session_shows_its_card_within_the_idle_bound() {
    let Some((site, _)) = quiet_site() else {
        return;
    };
    let (polled, _) = in_the_next(&site, Duration::from_secs(3));
    assert!(polled <= 1, "{polled} polls in 3 s: not the slow pace");
    assert!(site.fleet.cards().is_empty());
    // Asked with no hint from the gateway, as on a far machine whose
    // hints are missed: nothing tells of it but the next poll. And asked
    // just after a poll came back, which is the longest a card waits:
    // the whole of the slow pace, and the poll that finds it.
    for n in 0..3 {
        let before = polls(&site);
        until("a poll at the slow pace", || polls(&site) > before);
        let asked = Instant::now();
        let prompt = format!("p-{n}");
        site.gateway.asks_quietly("default", STUCK, &prompt);
        until("its card shows", || !site.fleet.cards().is_empty());
        let took = asked.elapsed();
        let card = site.fleet.cards().remove(0);
        assert!(
            took <= IDLE_CARD,
            "the card took {} ms: over the 13 s of a gateway where nothing is busy",
            took.as_millis()
        );
        // Found by the poll after the one that had just come back: it
        // waited for the slow pace, and no quicker one was running.
        let slow = Duration::from_millis(IDLE_POLL_MS as u64 / 2);
        assert!(took >= slow, "the card took {} ms", took.as_millis());
        println!("card {n} in the quiet session: {} ms", took.as_millis());
        // Settled elsewhere and its turn over, the session is listed as
        // working again and stands still: quiet once more.
        let settled = json!({"prompt_id": prompt});
        site.gateway
            .tells_quietly("default", STUCK, "clarify.settled", settled);
        until("the card closes", || site.fleet.cards().is_empty());
        assert!(card["id"].is_string(), "{card}");
        std::thread::sleep(QUIET + Duration::from_secs(3));
        let (polled, _) = in_the_next(&site, Duration::from_secs(3));
        assert!(polled <= 1, "{polled} polls in 3 s after card {n}");
    }
}
