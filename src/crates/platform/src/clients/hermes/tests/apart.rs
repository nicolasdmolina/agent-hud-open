//! A session whose events are slow to read is read apart from the polls
//! (ticket 30, after its first review): the polls leave it out and stay
//! quick, its own lane opens and closes its cards, and the lane ends
//! with its follower, with the session going idle, and with the reads
//! being quick again.
use super::{
    fake::{Fake, PROFILE},
    lab, marked, plain, quick, until, Lab, Pace,
};
use serde_json::{json, Value};
use std::time::Duration;

const SLOW: &str = "s-m-slow";
const OTHER: &str = "s-a-other";
/// How long the gateway holds back each read of the slow session's
/// events, and the read time from which a session is read apart.
const HELD: Duration = Duration::from_millis(400);
const APART: Duration = Duration::from_millis(150);

fn pace() -> Pace {
    Pace {
        apart: APART,
        ..quick()
    }
}

/// A gateway with two sessions in a turn, one of them slow to read, and
/// a follower that has found that out.
fn slowed() -> (Lab, Fake) {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    for session in [SLOW, OTHER] {
        gateway.tells_quietly(PROFILE, session, "message.start", json!({}));
    }
    gateway.world().slow_replay = Some((SLOW.into(), HELD));
    lab.follow_at(pace());
    until("the slow session is read apart", || lanes(&lab) == 1);
    (lab, gateway)
}

fn lanes(lab: &Lab) -> usize {
    lab.observer.as_ref().unwrap().local().apart.count()
}

/// The reads of a session's events the gateway got, with the socket each
/// came on.
fn reads(gateway: &Fake, session: &str) -> Vec<usize> {
    let calls = gateway.world().calls("session.events.since");
    let of = |(_, frame): &&(usize, Value)| frame["params"]["session_id"] == session;
    calls.iter().filter(of).map(|(socket, _)| *socket).collect()
}

fn shows(lab: &Lab, prompt: &str) -> bool {
    let ends = format!(":{prompt}");
    let its = |card: &Value| card["id"].as_str().is_some_and(|id| id.ends_with(&ends));
    lab.cards().iter().any(its)
}

fn settle(gateway: &Fake, session: &str, prompt: &str) {
    let payload = json!({"prompt_id": prompt, "execution_generation": 1});
    gateway.tells_quietly(PROFILE, session, "clarify.settled", payload);
}

#[test]
fn a_session_slow_to_read_is_read_apart_and_the_polls_go_on_without_it() {
    let (lab, gateway) = slowed();
    assert!(lab
        .log()
        .contains(&format!("apart session=hermes:{PROFILE}:{SLOW}")));
    // From now on the polls' socket carries no read of the slow session,
    // and goes on reading the other.
    let before = (reads(&gateway, SLOW), reads(&gateway, OTHER));
    let polls = gateway.world().listed.len();
    until("twenty more polls", || {
        gateway.world().listed.len() >= polls + 20
    });
    let on_polls = |sockets: &[usize]| sockets.iter().filter(|socket| **socket == 1).count();
    let after = (reads(&gateway, SLOW), reads(&gateway, OTHER));
    assert_eq!(on_polls(&after.0), on_polls(&before.0), "{:?}", after.0);
    assert!(on_polls(&after.1) >= on_polls(&before.1) + 20);
    // It is read on a socket of its own, which subscribes to nothing.
    let lane = *after.0.last().unwrap();
    assert_ne!(lane, 1);
    assert!(
        after.0.len() > before.0.len(),
        "the lane goes on reading it"
    );
    assert!(gateway.world().calls("session.resume").is_empty());
    assert_eq!(
        gateway.world().sockets,
        2,
        "the polls' socket and the lane's"
    );

    // Twenty polls took less than one read of the slow session held.
    // (Each is some milliseconds; the whole of them is held to this.)
    let began = std::time::Instant::now();
    let polls = gateway.world().listed.len();
    until("five more polls", || {
        gateway.world().listed.len() >= polls + 5
    });
    assert!(began.elapsed() < HELD * 4, "{:?}", began.elapsed());

    // A request in the other session shows at once; one in the slow
    // session shows through its lane, and goes when it is settled.
    gateway.asks_quietly(PROFILE, OTHER, "p-other");
    until("the other session's card", || shows(&lab, "p-other"));
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    until("the slow session's card", || shows(&lab, "p-slow"));
    assert_eq!(lab.cards().len(), 2);
    settle(&gateway, SLOW, "p-slow");
    until("the slow session's card goes", || !shows(&lab, "p-slow"));
    assert!(shows(&lab, "p-other"));
    assert_eq!(on_polls(&reads(&gateway, SLOW)), on_polls(&before.0));
}

#[test]
fn a_card_of_a_session_read_apart_is_answered_and_does_not_come_back() {
    let (lab, gateway) = slowed();
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    until("the slow session's card", || shows(&lab, "p-slow"));
    let card = lab.card();
    let id = card["id"].as_str().unwrap().to_owned();
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    let decision = json!({"kind": "answer", "answers": {key: {"selected": ["one"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    // A reading of the session that was under way when Hermes took the
    // answer still saw the request waiting. It counts for nothing.
    let limit = std::time::Instant::now() + HELD * 5;
    while std::time::Instant::now() < limit {
        assert!(!shows(&lab, "p-slow"), "the answered card came back");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
}

#[test]
fn the_polls_take_a_session_back_once_its_reads_are_quick_again() {
    let (lab, gateway) = slowed();
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    until("the slow session's card", || shows(&lab, "p-slow"));
    gateway.world().slow_replay = None;
    until("its lane ends", || lanes(&lab) == 0);
    let said = format!("together session=hermes:{PROFILE}:{SLOW} why=quick");
    assert!(lab.log().contains(&said), "{}", lab.log());
    // The polls read it again, and its card is the one it had.
    let on_polls = |gateway: &Fake| {
        let sockets = reads(gateway, SLOW);
        sockets.iter().filter(|socket| **socket == 1).count()
    };
    let before = on_polls(&gateway);
    until("the polls read it", || on_polls(&gateway) > before + 4);
    assert!(shows(&lab, "p-slow"));
    assert_eq!(lab.cards().len(), 1);
    settle(&gateway, SLOW, "p-slow");
    until("its card goes", || lab.cards().is_empty());
    assert_eq!(lanes(&lab), 0);
}

#[test]
fn a_lane_ends_when_the_polls_no_longer_want_the_session_read() {
    let (lab, gateway) = slowed();
    // The turn ends with nothing waiting: a poll would not read it.
    gateway.tells_quietly(PROFILE, SLOW, "message.complete", json!({}));
    until("its lane ends", || lanes(&lab) == 0);
    let said = format!("together session=hermes:{PROFILE}:{SLOW} why=unwanted");
    assert!(lab.log().contains(&said), "{}", lab.log());
    let read = reads(&gateway, SLOW).len();
    std::thread::sleep(HELD * 2);
    assert_eq!(reads(&gateway, SLOW).len(), read, "read no more");
}

#[test]
fn a_lane_ends_with_its_follower_and_asks_nothing_more() {
    let (mut lab, gateway) = slowed();
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    until("the slow session's card", || shows(&lab, "p-slow"));
    let shared = lab.observer.as_ref().unwrap().local();
    lab.observer.take();
    assert!(lab.cards().is_empty(), "withdrawn");
    until("its lane ends", || shared.apart.count() == 0);
    // What the gateway got once the lane ended is all it gets.
    let received = gateway.world().received.len();
    std::thread::sleep(HELD * 3);
    assert_eq!(gateway.world().received.len(), received);
    assert!(lab.cards().is_empty());
    assert!(!lab.log().contains("together session="), "{}", lab.log());
}

#[test]
fn a_dropped_socket_marks_the_card_of_a_session_read_apart_and_a_reading_takes_the_mark_off() {
    let (lab, gateway) = slowed();
    gateway.asks_quietly(PROFILE, SLOW, "p-slow");
    until("the slow session's card", || shows(&lab, "p-slow"));
    let before = lab.card();
    gateway.cut();
    until("the loss is seen", || {
        lab.log().contains("connection failed")
    });
    // The same request, found waiting again by whoever reads the session
    // next: the same card, with no mark.
    until("the card is in reach again", || {
        lab.cards().len() == 1 && !lab.cards().iter().any(marked)
    });
    assert_eq!(plain(lab.card()), before);
    assert_eq!(lab.log().matches("open id=").count(), 1, "registered once");
}

#[test]
fn no_more_than_four_sessions_of_one_gateway_are_read_apart() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let sessions = ["s-1", "s-2", "s-3", "s-4", "s-5", "s-6"];
    for session in sessions {
        gateway.tells_quietly(PROFILE, session, "message.start", json!({}));
    }
    // Every read of events is slow: each session's own read is.
    gateway.world().slow = APART + Duration::from_millis(60);
    lab.follow_at(pace());
    until("four are read apart", || lanes(&lab) == 4);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(lanes(&lab), 4);
    assert_eq!(
        lab.log().matches("apart session=").count(),
        4,
        "{}",
        lab.log()
    );
    // The two that stayed are still read by the polls.
    let read_by_polls = |session: &str| reads(&gateway, session).contains(&1);
    assert_eq!(
        sessions
            .iter()
            .filter(|session| read_by_polls(session))
            .count(),
        6
    );
}
