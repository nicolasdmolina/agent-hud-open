//! A set of questions asked in a quiet session (ticket 73), in the lab
//! of `quiet_slow.rs`. This is the probe of ticket 72's verifier, made
//! tests.
//!
//! Hermes goes on listing a session as working while a set of questions
//! waits in it (ticket 11), and the set may open with no event on the
//! stream. So nothing tells a poll that the session has changed: it
//! stays one of those whose read is asked last, and the set is found in
//! the answer to that read alone. The read is to be answered to its end
//! whatever else is asked last beside it: a look at an idle session
//! that is never answered, or another quiet session's read that is slow
//! or is never answered. The card is to show within spec section 10's
//! bound for a gateway where nothing is busy, thirteen seconds.
//!
//! The waits are the app's own, as in `quiet_slow.rs`; only the quiet
//! time is shorter.
use super::{
    history::Site,
    quiet::{three, IDLE_CARD, QUIET, STUCK, WORKED},
    quiet_slow::{app, listed, said, took, HELD},
    until,
};
use agent_hud_agents::hermes::gateway::Profile;
use serde_json::json;
use std::time::{Duration, Instant};

/// One more session listed as working for days. Its id sorts before
/// the stuck one's, so each is tried on both sides of the other.
const OTHER: &str = "a-stuck";

/// As `quiet_slow.rs` lists, and the other session listed as working.
fn two_working(gateway: &super::Fake) {
    listed(gateway);
    let mut world = gateway.world();
    let stream = world.stream_in("default", OTHER);
    stream.epoch = "epoch-1".into();
    stream.running = true;
}

/// The site once nothing has moved in it for the quiet time.
fn quiet_site() -> Option<Site> {
    let site = Site::paced(app(), three, two_working)?;
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
    Some(site)
}

/// What one set of questions met.
struct Met {
    /// From the set opening to its card showing.
    card: Duration,
    /// The slowest poll that ended meanwhile.
    held: Duration,
    /// How many sessions began to be read apart meanwhile.
    lanes: usize,
}

/// Opens a set of questions in `session` 150 ms after a poll began, so
/// that poll has read the session as it was and only the next one, a
/// whole slow pace later, can find the set. `then` is done at that same
/// moment. `moves` is whether an event is published with the set, as
/// the one real recording has it (`tool.start`), or the set opens with
/// the stream where it was. No hint is given either way. The set is
/// closed again after, and the gateway left until it is quiet.
fn opens(site: &Site, n: usize, session: &str, moves: bool, then: impl FnOnce()) -> Met {
    let begun = |site: &Site| site.gateway.world().listed.len();
    let before = begun(site);
    until("a poll begins", || begun(site) > before);
    std::thread::sleep(Duration::from_millis(150));
    let (polled, apart) = (took(site).len(), said(site, " apart session="));
    then();
    let asked = Instant::now();
    if moves {
        site.gateway
            .tells_quietly("default", session, "tool.start", json!({}));
    }
    let question = json!({"qid": "q0", "question": "Synthetic first",
        "choices": ["a", "b"], "multi_select": false});
    let set = json!({"id": format!("srq-set-{n}"), "method": "clarify", "params": {
        "session_id": session, "questions": [question], "answers": {}}});
    site.gateway.world().stream_in("default", session).open = vec![set];
    until("its card shows", || !site.fleet.cards().is_empty());
    let card = asked.elapsed();
    // A poll says how long it took a moment after the model has it.
    until("a poll since is in the log", || took(site).len() > polled);
    let slowest = took(site).into_iter().skip(polled).max().unwrap_or(0);
    let met = Met {
        card,
        held: Duration::from_millis(slowest),
        lanes: said(site, " apart session=") - apart,
    };
    {
        let mut world = site.gateway.world();
        world.stream_in("default", session).open = Vec::new();
        world.mute = None;
        world.slow_replay = None;
    }
    until("the card closes", || site.fleet.cards().is_empty());
    until("no session is read apart", || {
        said(site, " apart session=") == said(site, " together session=")
    });
    std::thread::sleep(QUIET + Duration::from_secs(9));
    met
}

fn in_time(what: &str, met: &Met) {
    println!(
        "a set of questions, {what}: card {} ms, the slowest poll {} ms, {} read apart",
        met.card.as_millis(),
        met.held.as_millis(),
        met.lanes
    );
    assert!(
        met.card <= IDLE_CARD,
        "{what}: the card took {} ms, over the 13 s",
        met.card.as_millis()
    );
}

/// The verifier's case: a look at an idle session goes unanswered in
/// the very poll that reads the set. It measured 19.9 s where the
/// stream stood still, and 11.9 s where an event came with the set.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_set_of_questions_in_a_quiet_session_is_in_time_beside_a_look_that_is_never_answered() {
    let Some(site) = quiet_site() else {
        return;
    };
    let met = opens(&site, 0, STUCK, false, || {});
    in_time("nothing slow, the stream where it was", &met);
    assert!(met.held < HELD, "a poll took {} ms", met.held.as_millis());

    // Another session each time: one whose look went unanswered rests.
    // The asker sorts after the session that says nothing, and before.
    for (n, (asker, mute, moves)) in [
        (STUCK, "s-old-9", false),
        (OTHER, "s-old-8", false),
        (STUCK, "t-old-2", true),
    ]
    .into_iter()
    .enumerate()
    {
        let met = opens(&site, n + 1, asker, moves, || {
            site.gateway.world().mute = Some(mute.to_owned());
        });
        let what = format!(
            "in {asker}, a look at {mute} never answered, the stream {}",
            if moves { "moved" } else { "where it was" }
        );
        in_time(&what, &met);
        // The look went unanswered in the poll that found the set, and
        // nobody had to be read apart for the card to show.
        assert!(
            met.held >= HELD,
            "no poll was held: {} ms",
            met.held.as_millis()
        );
        assert_eq!(met.lanes, 0, "{what}: {}", site.fleet.log());
    }
}

/// Another quiet session's read is what is slow: by four seconds, or it
/// is never answered at all. A gateway answers the calls of one socket
/// in turn, so the asker's own read may be held behind that one. It is
/// then read apart, on a socket of its own, and its card is in time all
/// the same, whichever of the two sorts first.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_set_of_questions_in_a_quiet_session_is_in_time_behind_another_quiet_sessions_read() {
    let Some(site) = quiet_site() else {
        return;
    };
    let slow = Duration::from_secs(4);
    for (n, (asker, other, moves)) in [
        (STUCK, OTHER, false),
        (STUCK, OTHER, true),
        (OTHER, STUCK, false),
    ]
    .into_iter()
    .enumerate()
    {
        let met = opens(&site, n, asker, moves, || {
            site.gateway.world().slow_replay = Some((other.to_owned(), slow));
        });
        let what = format!(
            "in {asker}, the read of {other} slow by 4 s, the stream {}",
            if moves { "moved" } else { "where it was" }
        );
        in_time(&what, &met);
        assert!(
            met.held >= HELD,
            "no poll was held: {} ms",
            met.held.as_millis()
        );
    }
    // The other session says nothing at all when it is asked where its
    // stream is, so the round that asks both never ends by itself, and
    // the asker's next read is never sent in that poll.
    for (n, (asker, other)) in [(OTHER, STUCK), (STUCK, OTHER)].into_iter().enumerate() {
        let met = opens(&site, n + 3, asker, false, || {
            site.gateway.world().mute = Some(other.to_owned());
        });
        let what = format!("in {asker}, the read of {other} never answered");
        in_time(&what, &met);
        assert!(
            met.held >= HELD,
            "no poll was held: {} ms",
            met.held.as_millis()
        );
    }
}
