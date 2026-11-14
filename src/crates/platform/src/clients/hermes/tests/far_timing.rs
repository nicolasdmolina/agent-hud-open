//! How long a far machine's request takes to show, from Hermes asking to
//! the card being in core, held against the bounds of the phase 2 design
//! (spec section 10). One suite, kept so that a later ticket that adds
//! reads of its own can run it unchanged and see that the bounds hold.
//!
//! Everything is as in the app: the app's own pace (`Pace::default()`),
//! the route over SSH through the lab's `sshd`, a relay for each ticket.
//! The gateway's hints are withheld: nothing here ever says
//! `sessions.changed`, so only the polls find what was asked.
//!
//! A miss fails the test. Each prints its numbers, as one line that
//! begins with `TIMING`. They wait on the clock, some for a minute, so
//! they are run by hand and not with every test run, on a machine that
//! is not starved by other builds:
//! `cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_timing -- --ignored --nocapture --test-threads 3`.
use super::{
    super::Pace,
    fake::Fake,
    far::{far_lab, until, FarLab},
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// A card in a session that was working or waiting, or beside a card
/// that shows: 19 asks in 20 within this.
const BUSY: Duration = Duration::from_secs(4);
/// And none of them over this.
const BUSY_MOST: Duration = Duration::from_secs(6);
/// A card when every listed session was idle, or none was listed.
const IDLE: Duration = Duration::from_secs(13);
/// A request no read shows: a snapshot, taken every 30 s, and one poll.
const HIDDEN: Duration = Duration::from_secs(32);
/// One whole poll over the link, a slow replay in it or not: 19 polls in
/// 20 within this.
const POLL: Duration = Duration::from_secs(1);
const PROFILE: &str = "default";

fn following(pace: Pace) -> Option<(FarLab, Fake)> {
    let mut far = far_lab()?;
    let gateway = Fake::serving(&far.root, "instance-1", &[PROFILE]);
    gateway.world().install = Some(super::far::INSTALL.into());
    far.follow_at(pace);
    far.followed();
    until("the polls run", || !gateway.world().listed.is_empty());
    Some((far, gateway))
}

/// A session in a turn, with no hint that it began.
fn works(gateway: &Fake, session: &str) {
    gateway.tells_quietly(PROFILE, session, "message.start", json!({}));
}

/// How many polls the gateway has seen begin.
fn polls(gateway: &Fake) -> usize {
    gateway.world().listed.len()
}

fn shows(far: &FarLab, prompt: &str) -> bool {
    let ends = format!(":{prompt}");
    let its = |card: &Value| card["id"].as_str().is_some_and(|id| id.ends_with(&ends));
    far.cards().iter().any(its)
}

/// Asks in `session`, with no hint, and says how long the card took.
fn asked(far: &FarLab, gateway: &Fake, session: &str, prompt: &str, limit: Duration) -> Duration {
    let began = Instant::now();
    gateway.asks_quietly(PROFILE, session, prompt);
    while !shows(far, prompt) {
        let took = began.elapsed();
        assert!(took <= limit, "no card for {prompt} after {took:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
    began.elapsed()
}

/// The request is settled on the far side, and its card goes.
fn settled(far: &FarLab, gateway: &Fake, session: &str, prompt: &str) {
    let payload = json!({"prompt_id": prompt, "execution_generation": 1});
    gateway.tells_quietly(PROFILE, session, "clarify.settled", payload);
    until("the settled card goes", || !shows(far, prompt));
}

/// Prints the numbers of one case, and fails it where more than `spare`
/// of them are over `bound` or any is over `most`.
fn held(name: &str, mut took: Vec<Duration>, bound: Duration, most: Duration, spare: usize) {
    took.sort();
    let ms = |time: &Duration| time.as_millis();
    let over = took.iter().filter(|time| **time > bound).count();
    let (least, middle, longest) = (&took[0], &took[took.len() / 2], &took[took.len() - 1]);
    println!(
        "TIMING {name} n={} least_ms={} middle_ms={} most_ms={} over_{}ms={over} \
         bound_ms={} never_over_ms={}",
        took.len(),
        ms(least),
        ms(middle),
        ms(longest),
        ms(&bound),
        ms(&bound),
        ms(&most),
    );
    assert!(
        over <= spare,
        "{name}: {over} of {} over {bound:?}",
        took.len()
    );
    assert!(longest <= &most, "{name}: {longest:?} is over {most:?}");
}

/// Twenty asks, each at another moment between two polls: the wait
/// before each grows by a twentieth of `cycle`, so the asks fall evenly
/// over the time from one poll to the next, its worst moment included.
fn twenty(
    far: &FarLab,
    gateway: &Fake,
    sessions: &[&str],
    cycle: Duration,
    tag: &str,
) -> Vec<Duration> {
    let mut took = Vec::new();
    for ask in 0..20u32 {
        // From the start of a poll, so the moment asked at is known.
        let seen = polls(gateway);
        until("the next poll begins", || polls(gateway) > seen);
        std::thread::sleep(cycle * ask / 20);
        let session = sessions[ask as usize % sessions.len()];
        let prompt = format!("{tag}-{ask}");
        took.push(asked(far, gateway, session, &prompt, BUSY_MOST * 2));
        settled(far, gateway, session, &prompt);
    }
    took
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_in_a_busy_session() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    works(&gateway, "s-busy");
    let seen = polls(&gateway);
    until("a poll lists it working", || polls(&gateway) > seen + 1);
    let took = twenty(&far, &gateway, &["s-busy"], Duration::from_secs(1), "busy");
    held("card_in_a_busy_session", took, BUSY, BUSY_MOST, 1);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_beside_a_showing_card() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    // One card shows the whole time, in a session of its own.
    asked(&far, &gateway, "s-shown", "shown", BUSY_MOST * 2);
    let mut took = Vec::new();
    for ask in 0..20u32 {
        let seen = polls(&gateway);
        until("the next poll begins", || polls(&gateway) > seen);
        std::thread::sleep(Duration::from_secs(1) * ask / 20);
        // A session that ran no turn until now: it begins one and asks.
        let (session, prompt) = (format!("s-new-{ask}"), format!("beside-{ask}"));
        took.push(asked(&far, &gateway, &session, &prompt, BUSY_MOST * 2));
        assert!(shows(&far, "shown"));
        settled(&far, &gateway, &session, &prompt);
        gateway.tells_quietly(PROFILE, &session, "message.complete", json!({}));
    }
    held("card_beside_a_showing_card", took, BUSY, BUSY_MOST, 1);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_in_an_idle_listed_session() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    // A turn that came and went: the session is listed, idle, from now.
    works(&gateway, "s-idle");
    gateway.tells_quietly(PROFILE, "s-idle", "message.complete", json!({}));
    let listed = Instant::now();
    let before = polls(&gateway);
    std::thread::sleep(Duration::from_secs(60));
    let polled = polls(&gateway) - before;
    assert!(far.cards().is_empty());
    let took = asked(&far, &gateway, "s-idle", "idle", IDLE * 2);
    println!(
        "TIMING card_in_an_idle_listed_session listed_idle_for_ms={} polls_meanwhile={polled}",
        listed.elapsed().as_millis()
    );
    held("card_in_an_idle_listed_session", vec![took], IDLE, IDLE, 0);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_when_nothing_was_listed() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    let mut took = Vec::new();
    // Just after a poll that found nothing, which is the longest wait,
    // then a third and two thirds of the way to the next.
    for (ask, after) in [0, 3, 6].into_iter().enumerate() {
        // Two polls in a row with nothing listed: the slow pace is on.
        let seen = polls(&gateway);
        until("two polls find nothing", || polls(&gateway) >= seen + 2);
        std::thread::sleep(Duration::from_millis(100) + Duration::from_secs(after));
        let (session, prompt) = (format!("s-first-{ask}"), format!("nothing-{ask}"));
        took.push(asked(&far, &gateway, &session, &prompt, IDLE * 2));
        settled(&far, &gateway, &session, &prompt);
        gateway.world().streams.clear();
    }
    held("card_when_nothing_was_listed", took, IDLE, IDLE, 0);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_with_five_busy_sessions() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    let sessions = ["s-one", "s-two", "s-three", "s-four", "s-five"];
    for session in sessions {
        works(&gateway, session);
    }
    let seen = polls(&gateway);
    until("a poll lists them working", || polls(&gateway) > seen + 1);
    let took = twenty(&far, &gateway, &sessions, Duration::from_secs(1), "five");
    held("card_with_five_busy_sessions", took, BUSY, BUSY_MOST, 1);
}

/// How long the slow session's events take to read, each time.
const SLOW: Duration = Duration::from_secs(2);

#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_behind_a_slow_replay() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    // The session whose events are slow to read sorts between the two
    // that ask, so one is read before it in a poll and one after.
    for session in ["s-a-early", "s-m-slow", "s-z-late"] {
        works(&gateway, session);
    }
    gateway.world().slow_replay = Some(("s-m-slow".into(), SLOW));
    let seen = polls(&gateway);
    until("polls run behind the slow read", || {
        polls(&gateway) > seen + 1
    });
    // No allowance for the slow read: the same bounds as any busy session.
    let asking = ["s-a-early", "s-z-late"];
    let took = twenty(&far, &gateway, &asking, SLOW, "slow");
    held("card_behind_a_slow_replay", took, BUSY, BUSY_MOST, 1);
}

/// The session that is slow to read has cards of its own. Its events
/// are read apart from the polls, one reading after another, so a card
/// of it waits for that read and no longer: held to the bound of any
/// busy session.
#[test]
#[ignore = "waits on the clock: run by hand"]
fn card_in_the_session_that_is_slow_to_read() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    for session in ["s-a-early", "s-m-slow"] {
        works(&gateway, session);
    }
    gateway.world().slow_replay = Some(("s-m-slow".into(), SLOW));
    until("the slow session is read apart", || {
        far.log().contains("apart session=")
    });
    let took = twenty(&far, &gateway, &["s-m-slow"], SLOW, "own");
    let name = "card_in_the_session_that_is_slow_to_read";
    held(name, took, BUSY, BUSY_MOST, 1);
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn a_request_no_read_shows() {
    let Some((far, gateway)) = following(Pace::default()) else {
        return;
    };
    // A request that waits and that no event tells of: only a snapshot
    // lists it. The first is found by the poll that first lists its
    // session, which takes the session's first snapshot at once.
    let hidden = |prompt: &str| {
        let mut world = gateway.world();
        let stream = world.stream_in(PROFILE, "s-hidden");
        stream.epoch = "epoch-1".into();
        stream.running = true;
        stream
            .prompts
            .push(json!({"kind": "clarify", "prompt_id": prompt,
            "execution_generation": 1, "question": "Which one?", "choices": ["one", "two"],
            "multi_select": false, "timeout_s": 600}));
    };
    let began = Instant::now();
    hidden("hidden-first");
    until("the first shows", || shows(&far, "hidden-first"));
    let first = began.elapsed();
    let snapshots = |gateway: &Fake| gateway.world().calls("session.resume").len();
    assert_eq!(snapshots(&gateway), 1);

    // The second comes while the first still shows: the next snapshot of
    // that session is the one that finds it, 30 s after the last.
    std::thread::sleep(Duration::from_secs(3));
    let began = Instant::now();
    hidden("hidden-second");
    while !shows(&far, "hidden-second") {
        let took = began.elapsed();
        assert!(took <= HIDDEN * 2, "no card after {took:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    let took = began.elapsed();
    assert_eq!(snapshots(&gateway), 2, "found by the one snapshot");
    until("the snapshot's socket is done with", || {
        gateway.world().visit_over()
    });
    assert!(!far.log().contains("visit failed"), "{}", far.log());
    // Nothing was listed before the first, so the slow pace found it.
    println!(
        "TIMING a_request_no_read_shows first_found_by_the_next_poll_ms={} \
         second_asked_ms_after_the_first_showed=3000",
        first.as_millis()
    );
    held("a_request_no_read_shows", vec![took], HIDDEN, HIDDEN, 0);
}

/// What the log says each poll took, in order.
fn poll_times(far: &FarLab) -> Vec<Duration> {
    let log = far.log();
    let took = |line: &str| {
        let ms = line.split("trace poll ms=").nth(1)?.split(' ').next()?;
        Some(Duration::from_millis(ms.parse().ok()?))
    };
    log.lines().filter_map(took).collect()
}

#[test]
#[ignore = "waits on the clock: run by hand"]
fn poll_time_over_the_link() {
    let pace = Pace {
        trace: true,
        ..Pace::default()
    };
    let Some((far, gateway)) = following(pace) else {
        return;
    };
    // Several busy sessions, one of them waiting on a card.
    for session in ["s-one", "s-two", "s-three", "s-four", "s-m-slow"] {
        works(&gateway, session);
    }
    asked(&far, &gateway, "s-one", "shown", BUSY_MOST * 2);
    let twenty_more = |far: &FarLab| {
        let from = poll_times(far).len();
        until("twenty more polls", || poll_times(far).len() >= from + 20);
        poll_times(far)[from..from + 20].to_vec()
    };
    held(
        "poll_time_over_the_link",
        twenty_more(&far),
        POLL,
        POLL * 3,
        1,
    );

    // With one session's events slow to read: every whole poll as the
    // connection times it, with nothing taken off. Counted from the
    // moment the read turns slow, so the poll that finds it out is one
    // of the twenty, and is the one a bound of 19 in 20 leaves room for.
    let from = poll_times(&far).len();
    gateway.world().slow_replay = Some(("s-m-slow".into(), SLOW));
    until("twenty polls from then", || {
        poll_times(&far).len() >= from + 20
    });
    let slow = poll_times(&far)[from..from + 20].to_vec();
    let met = slow.iter().any(|took| *took >= SLOW);
    assert!(met, "no poll met the slow read: {slow:?}");
    assert!(far.log().contains("apart session="), "{}", far.log());
    let name = "poll_time_over_the_link_with_a_slow_replay";
    held(name, slow, POLL, POLL * 3, 1);
    // And the twenty after those, with the slow session read apart.
    let name = "poll_time_over_the_link_beside_a_slow_replay";
    held(name, twenty_more(&far), POLL, POLL, 0);
}
