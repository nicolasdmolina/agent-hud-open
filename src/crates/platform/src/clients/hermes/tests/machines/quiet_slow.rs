//! A quiet far gateway with one call that is slow (ticket 72), in the
//! lab of `quiet.rs`. This is the probe of ticket 67's verifier, made
//! tests: a gateway where nothing has moved for the quiet time is polled
//! at the model's slow pace, a question is asked just after a poll
//! began, and one call of the poll that finds it is slow, or is never
//! answered. The card is to show within spec section 10's bound for a
//! gateway where nothing is busy, thirteen seconds, all the same.
//!
//! The waits are the app's own (`Pace::default()`): a poll every ten
//! seconds, a call that may take five. Only the quiet time is shorter,
//! four seconds for the app's thirty minutes, and the history's pace and
//! the look at the machine are the lab's.
use super::{
    history::Site,
    quiet::{as_the_mini_lists, paced, three, IDLE_CARD, QUIET, STUCK, WORKED},
    until,
};
use crate::clients::hermes::Pace;
use agent_hud_agents::hermes::gateway::Profile;
use serde_json::json;
use std::time::{Duration, Instant};

/// How long a poll takes that one slow call held up, at the least: every
/// other poll here takes a few milliseconds.
pub(super) const HELD: Duration = Duration::from_millis(1_500);

pub(super) fn app() -> Pace {
    Pace {
        quiet: QUIET,
        history: paced().history,
        trace: true,
        discover: Some(Duration::from_millis(500)),
        ..Pace::default()
    }
}

/// As the Mini lists (`quiet.rs`), and three idle chats more whose ids
/// sort after the stuck session's: a gateway answers the calls of one
/// socket in turn, so which side of a slow call a session's own read
/// falls on is not left to its name.
pub(super) fn listed(gateway: &super::Fake) {
    as_the_mini_lists(gateway);
    let mut world = gateway.world();
    for n in 0..3 {
        world.stream_in("default", &format!("t-old-{n}")).epoch = "epoch-1".into();
    }
}

/// The site once nothing has moved in it for the quiet time.
fn quiet_site() -> Option<Site> {
    let site = Site::paced(app(), three, listed)?;
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

/// How long each poll took that ran to its end, by the log.
pub(super) fn took(site: &Site) -> Vec<u64> {
    let log = site.fleet.log();
    log.lines()
        .filter_map(|line| line.split(" trace poll ms=").nth(1))
        .filter_map(|rest| rest.split(' ').next()?.parse().ok())
        .collect()
}

/// How often the log says `what`.
pub(super) fn said(site: &Site, what: &str) -> usize {
    site.fleet.log().matches(what).count()
}

/// Asks in `session` 150 ms after a poll began, with no hint, so that
/// poll has listed the session as it was and only the next one, a whole
/// slow pace later, can find the question. `then` is done at that same
/// moment. Returns how long the card took, and how long the slowest poll
/// took that ended meanwhile.
fn ask(site: &Site, session: &str, prompt: &str, then: impl FnOnce()) -> (Duration, Duration) {
    let begun = |site: &Site| site.gateway.world().listed.len();
    let before = begun(site);
    until("a poll begins", || begun(site) > before);
    std::thread::sleep(Duration::from_millis(150));
    let polled = took(site).len();
    then();
    let asked = Instant::now();
    site.gateway.asks_quietly("default", session, prompt);
    until("its card shows", || {
        let cards = site.fleet.cards();
        cards.iter().any(|card| card.to_string().contains(prompt))
    });
    let card = asked.elapsed();
    // A poll says how long it took a moment after the model has it.
    until("a poll since is in the log", || took(site).len() > polled);
    let slowest = took(site).into_iter().skip(polled).max().unwrap_or(0);
    (card, Duration::from_millis(slowest))
}

/// The question is settled elsewhere, and the gateway is left until
/// nothing has moved in it for the quiet time again and no session is
/// read apart.
fn settle(site: &Site, session: &str, prompt: &str) {
    let settled = json!({"prompt_id": prompt});
    site.gateway
        .tells_quietly("default", session, "clarify.settled", settled);
    until("the card closes", || site.fleet.cards().is_empty());
    until("no session is read apart", || {
        said(site, " apart session=") == said(site, " together session=")
    });
    std::thread::sleep(QUIET + Duration::from_secs(9));
}

/// A read of the session listed as working turns slow in the very poll
/// that finds another session's question: by four seconds, which the
/// verifier measured at 13.9 s, and by six, which is longer than any
/// call may take.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_card_on_a_quiet_gateway_is_in_time_behind_a_read_that_turns_slow() {
    let Some(site) = quiet_site() else {
        return;
    };
    let cases = [("s-old-0", 4), ("t-old-0", 4), ("t-old-1", 6)];
    for (n, (session, slow)) in cases.into_iter().enumerate() {
        let (prompt, slow) = (format!("p-{n}"), Duration::from_secs(slow));
        let (card, held) = ask(&site, session, &prompt, || {
            site.gateway.world().slow_replay = Some((STUCK.to_owned(), slow));
        });
        println!(
            "behind a read slow by {} s, asked in {session}: card {} ms, the poll {} ms",
            slow.as_secs(),
            card.as_millis(),
            held.as_millis()
        );
        // The slow read was in the poll that found the question.
        assert!(held >= HELD, "no poll was held: {} ms", held.as_millis());
        assert!(
            card <= IDLE_CARD,
            "the card took {} ms behind a read slow by {} s: over the 13 s",
            card.as_millis(),
            slow.as_secs()
        );
        site.gateway.world().slow_replay = None;
        settle(&site, session, &prompt);
    }
}

/// One idle session stops answering the looks at its stream, and the
/// poll that first meets that is the one that finds another session's
/// question. The verifier measured 14.9 s.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_card_on_a_quiet_gateway_is_in_time_beside_a_look_that_is_never_answered() {
    let Some(site) = quiet_site() else {
        return;
    };
    // Another session each time: one whose look went unanswered rests.
    let cases = [
        ("s-old-0", "s-old-9"),
        ("t-old-0", "s-old-8"),
        ("s-old-1", "t-old-2"),
    ];
    for (n, (session, mute)) in cases.into_iter().enumerate() {
        let prompt = format!("p-{n}");
        let (card, held) = ask(&site, session, &prompt, || {
            site.gateway.world().mute = Some(mute.to_owned());
        });
        println!(
            "beside a look at {mute} never answered, asked in {session}: card {} ms, the poll {} ms",
            card.as_millis(),
            held.as_millis()
        );
        assert!(held >= HELD, "no poll was held: {} ms", held.as_millis());
        assert!(
            card <= IDLE_CARD,
            "the card took {} ms beside a look never answered: over the 13 s",
            card.as_millis()
        );
        site.gateway.world().mute = None;
        settle(&site, session, &prompt);
    }
}

/// The session listed as working is slow to read at every read, so it
/// is read apart (`apart.rs`). While nothing moves, it is read no more
/// often than the gateway is polled, and its lane lives from one slow
/// poll to the next. Cards are in time beside it, and a question asked
/// in it shows a slow pace and one read of its own later.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_quiet_session_that_is_slow_to_read_is_read_apart_at_the_pace_of_the_polls() {
    let Some(site) = quiet_site() else {
        return;
    };
    let slow = Duration::from_secs(3);
    site.gateway.world().slow_replay = Some((STUCK.to_owned(), slow));
    until("it is read apart", || said(&site, " apart session=") == 1);
    std::thread::sleep(QUIET + Duration::from_secs(12));

    let (polled, read) = (took(&site).len(), said(&site, " trace apart ms="));
    std::thread::sleep(Duration::from_secs(30));
    let (polled, read) = (
        took(&site).len() - polled,
        said(&site, " trace apart ms=") - read,
    );
    println!("quiet and slow to read: {polled} polls and {read} reads apart in 30 s");
    assert!(polled <= 4, "{polled} polls in 30 s: not the slow pace");
    assert!(
        (2..=4).contains(&read),
        "{read} reads apart in 30 s of quiet, for a poll every 10 s"
    );
    // One lane all the while: it did not end between two slow polls.
    assert_eq!(said(&site, " apart session="), 1, "{}", site.fleet.log());
    assert_eq!(said(&site, " together session="), 0, "{}", site.fleet.log());

    // A question in another session: in time, with no poll held up.
    let (card, held) = ask(&site, "s-old-0", "p-other", || {});
    println!(
        "beside it: card {} ms, the poll {} ms",
        card.as_millis(),
        held.as_millis()
    );
    assert!(card <= IDLE_CARD, "the card took {} ms", card.as_millis());
    assert!(held < HELD, "a poll took {} ms", held.as_millis());
    let settled = json!({"prompt_id": "p-other"});
    site.gateway
        .tells_quietly("default", "s-old-0", "clarify.settled", settled);
    until("the card closes", || site.fleet.cards().is_empty());
    std::thread::sleep(QUIET + Duration::from_secs(12));

    // Something moves just after a read apart came back, so the next
    // one is a whole slow pace away: the lane does not sleep that out.
    // It reads at the quick pace from the poll that found the move.
    let read = said(&site, " trace apart ms=");
    until("a read apart comes back", || {
        said(&site, " trace apart ms=") > read
    });
    let read = said(&site, " trace apart ms=");
    site.gateway.asks("default", "s-old-1", "p-moved");
    std::thread::sleep(Duration::from_secs(8));
    let quick = said(&site, " trace apart ms=") - read;
    println!("once something moved: {quick} reads apart in 8 s");
    assert!(quick >= 2, "{quick} reads apart in the 8 s after a move");
    let settled = json!({"prompt_id": "p-moved"});
    site.gateway
        .tells_quietly("default", "s-old-1", "clarify.settled", settled);
    until("the card closes", || site.fleet.cards().is_empty());
    std::thread::sleep(QUIET + Duration::from_secs(12));

    // A question in the slow session itself is never lost: it shows
    // within the slow pace and its own read.
    let (card, _) = ask(&site, STUCK, "p-own", || {});
    println!("in the slow session itself: card {} ms", card.as_millis());
    assert!(
        card <= IDLE_CARD + slow,
        "the card took {} ms",
        card.as_millis()
    );
}
