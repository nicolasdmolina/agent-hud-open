//! A session whose events are slow to read is read apart from the polls
//! (ticket 30, after its first review): the polls leave it out and stay
//! quick, its own lane opens and closes its cards, and the lane ends
//! with its follower, with the session going idle, and with the reads
//! being quick again.
use super::super::{opening::opened, polls::rounds, Shared};
use super::{
    fake::{Fake, Hold, PROFILE},
    lab, marked, plain, quick, until, Lab, Pace,
};
use agent_hud_agents::hermes::gateway::{Connection, SessionRef, Status};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

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

/// The longest one of these tests may take. Alone each is over in a few
/// seconds; this leaves room for a machine busy with other builds.
const LIMIT: Duration = Duration::from_secs(90);

/// Runs a test's body and fails the test where it is not over within
/// `LIMIT`. Every wait in these tests has a limit of its own (`until`),
/// so a body that goes on past this one is stuck where nothing times
/// out: on a lock, or on a thread that never ends. Such a test says so
/// and fails, in place of holding the whole run up. The thread it was
/// stuck on is left behind and ends with the test program.
fn within_the_limit(body: impl FnOnce() + Send + 'static) {
    let (done, over) = std::sync::mpsc::channel();
    let ran = std::thread::spawn(move || {
        body();
        let _ = done.send(());
    });
    match over.recv_timeout(LIMIT) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("the test was not over within {LIMIT:?}: it hangs")
        }
        // The body failed: the test fails as it did.
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(failed) = ran.join() {
                std::panic::resume_unwind(failed);
            }
        }
    }
}

#[test]
fn a_session_slow_to_read_is_read_apart_and_the_polls_go_on_without_it() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        assert!(lab
            .log()
            .contains(&format!("apart session=hermes:{PROFILE}:{SLOW}")));
        // From now on the polls' socket carries no read of the slow session,
        // and goes on reading the other. Counted by the reads themselves,
        // as the gateway got them: twenty more of the other session on the
        // polls' socket, and in that time not one of the slow session there.
        let on_polls = |sockets: &[usize]| sockets.iter().filter(|socket| **socket == 1).count();
        let before = (reads(&gateway, SLOW), reads(&gateway, OTHER));
        until("the polls read the other session twenty times more", || {
            on_polls(&reads(&gateway, OTHER)) >= on_polls(&before.1) + 20
        });
        let after = reads(&gateway, SLOW);
        assert_eq!(on_polls(&after), on_polls(&before.0), "{after:?}");
        // It is read on a socket of its own, which subscribes to nothing.
        until("the lane goes on reading it", || {
            reads(&gateway, SLOW).len() > before.0.len()
        });
        let since = reads(&gateway, SLOW)[before.0.len()..].to_vec();
        assert!(since.iter().all(|socket| *socket != 1), "{since:?}");
        assert!(gateway.world().calls("session.resume").is_empty());

        // The polls do not wait for the slow read. While the slow session is
        // read three times more, each read held back by the gateway, the
        // polls run many times over; had they waited, there would be a poll
        // for each of those reads and no more. Counted, not clocked, so a
        // machine busy with other builds changes nothing but the margin.
        let lists = gateway.world().listed.len();
        let read = reads(&gateway, SLOW).len();
        // A reading is two reads: where the stream is, then its events.
        until("three more readings of the slow session", || {
            reads(&gateway, SLOW).len() >= read + 6
        });
        // And a poll is two lists, one for each profile.
        let polls = (gateway.world().listed.len() - lists) / 2;
        assert!(polls >= 5, "{polls} polls beside three slow readings");

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
    });
}

#[test]
fn a_card_of_a_session_read_apart_is_answered_and_does_not_come_back() {
    within_the_limit(|| {
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
    });
}

#[test]
fn the_polls_take_a_session_back_once_its_reads_are_quick_again() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        gateway.asks_quietly(PROFILE, SLOW, "p-slow");
        until("the slow session's card", || shows(&lab, "p-slow"));
        gateway.world().slow_replay = None;
        until("its lane ends", || lanes(&lab) == 0);
        // The lane leaves the count a moment before it writes why.
        let said = format!("together session=hermes:{PROFILE}:{SLOW} why=quick");
        let wrote = within(Duration::from_secs(10), || lab.log().contains(&said));
        assert!(wrote, "{}", lab.log());
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
    });
}

#[test]
fn a_lane_ends_when_the_polls_no_longer_want_the_session_read() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        // The turn ends with nothing waiting: a poll would not read it.
        gateway.tells_quietly(PROFILE, SLOW, "message.complete", json!({}));
        until("its lane ends", || lanes(&lab) == 0);
        // The lane leaves the count a moment before it writes why.
        let said = format!("together session=hermes:{PROFILE}:{SLOW} why=unwanted");
        let wrote = within(Duration::from_secs(10), || lab.log().contains(&said));
        assert!(wrote, "{}", lab.log());
        let read = reads(&gateway, SLOW).len();
        std::thread::sleep(HELD * 2);
        assert_eq!(reads(&gateway, SLOW).len(), read, "read no more");
    });
}

#[test]
fn a_lane_ends_with_its_follower_and_asks_nothing_more() {
    within_the_limit(|| {
        // The slow session alone, so every read of events is its lane's.
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        gateway.tells_quietly(PROFILE, SLOW, "message.start", json!({}));
        gateway.world().slow_replay = Some((SLOW.into(), HELD));
        lab.follow_at(pace());
        until("the slow session is read apart", || lanes(&lab) == 1);
        gateway.asks_quietly(PROFILE, SLOW, "p-slow");
        until("the slow session's card", || shows(&lab, "p-slow"));

        // The lane's next reading is held at its first reply, with its read
        // of the events still to ask, and the follower ends meanwhile:
        // client hooks were turned off.
        let held = Arc::new(Hold::default());
        gateway.world().hold_next = Some(("stream-probe", held.clone()));
        until("a read of the lane's is held", || {
            held.entered.load(Ordering::Acquire)
        });
        let shared = lab.observer.as_ref().unwrap().local();
        lab.observer.take();
        assert!(lab.cards().is_empty(), "withdrawn");
        let asked = gateway.world().received.len();
        held.released.store(true, Ordering::Release);

        // The reply comes back to a lane whose follower ended: it sends no
        // next call, ends, and brings no card back.
        until("its lane ends", || shared.apart.count() == 0);
        std::thread::sleep(HELD * 3);
        let world = gateway.world();
        let after: Vec<&Value> = world.received[asked..]
            .iter()
            .map(|(_, frame)| frame)
            .collect();
        assert_eq!(
            after,
            Vec::<&Value>::new(),
            "asked after its follower ended"
        );
        drop(world);
        assert!(lab.cards().is_empty());
        assert!(!lab.log().contains("together session="), "{}", lab.log());
    });
}

#[test]
fn a_dropped_socket_marks_the_card_of_a_session_read_apart_and_a_reading_takes_the_mark_off() {
    within_the_limit(|| {
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
    });
}

#[test]
fn no_more_than_four_sessions_of_one_gateway_are_read_apart() {
    within_the_limit(|| {
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
    });
}

/// The independent verifier's probe from the second review of ticket 30
/// (`evidence/30/verifier2-files/inflight-unlisted-probe.rs`), as written
/// but for its layout. A reading of the slow session is held at its
/// first reply after the polls listed the session waiting on a request
/// no event shows, which calls for a snapshot. The session then leaves
/// the live list, the model is seen to know it, and the reply is let
/// go. The lane went on to `session.resume` a session off the live list.
#[test]
fn reviewer30_an_inflight_lane_never_resumes_after_the_latest_poll_unlists_its_session() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        let shared = lab.observer.as_ref().unwrap().local();
        let held = Arc::new(Hold::default());
        {
            let mut world = gateway.world();
            world.hold_next = Some(("slow-probe", held.clone()));
        }
        until("the lane's probe is held", || {
            held.entered.load(Ordering::Acquire)
        });
        // This reading definitely captured Working before the hidden prompt.
        gateway
            .world()
            .stream_in(PROFILE, SLOW)
            .prompts
            .push(json!({"kind":"clarify",
            "prompt_id":"p-hidden", "execution_generation":1, "question":"Synthetic?",
            "choices":["one"], "multi_select":false, "timeout_s":600}));
        let waiting = agent_hud_agents::hermes::gateway::Status::Waiting;
        until("the live list says waiting", || {
            let statuses = shared.model().gateway.statuses();
            statuses
                .iter()
                .any(|(s, status)| s.id() == SLOW && *status == waiting)
        });
        // The first reading may have begun while still working. It must end
        // before we hold a reading definitely begun with the waiting status.
        let held_next = Arc::new(Hold::default());
        gateway.world().hold_next = Some(("slow-probe", held_next.clone()));
        held.released.store(true, Ordering::Release);
        until("the next lane probe is held", || {
            held_next.entered.load(Ordering::Acquire)
        });
        gateway.world().unlisted.push(SLOW.into());
        until("the latest poll removed the session", || {
            let statuses = shared.model().gateway.statuses();
            !statuses.iter().any(|(s, _)| s.id() == SLOW)
        });
        let before = gateway.world().calls("session.resume").len();
        held_next.released.store(true, Ordering::Release);
        until("the lane finishes", || shared.apart.count() == 0);
        let calls = gateway.world().calls("session.resume");
        let after = calls.len();
        let resumed: Vec<Value> = calls[before..]
            .iter()
            .map(|(_, frame)| frame["params"]["session_id"].clone())
            .collect();
        println!(
            "TRACE unlisted lane before_resumes={before} after_resumes={after} live_slow=false \
             resumed_sessions={resumed:?}"
        );
        assert_eq!(
            after, before,
            "a lane resumed a session after a newer live list removed it"
        );
    });
}

/// A reading that counts no more sends nothing further, whatever made it
/// so, and is not waited for. Here a reading begun while the session
/// waits is held at its first reply, the request is settled on the far
/// side, and the polls then list the session as working. The reading is
/// given up while its reply is still held: the next one, begun under the
/// new listing on a socket of its own, takes the card down. Once the
/// held reply is let go, the reading that was given up never asks for
/// the events.
#[test]
fn a_reading_that_counts_no_more_is_given_up_while_its_reply_is_held_and_asks_nothing_further() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        gateway.asks_quietly(PROFILE, SLOW, "p-slow");
        until("the slow session's card", || shows(&lab, "p-slow"));
        // Two polls more, so that a poll has listed it waiting.
        let lists = gateway.world().listed.len();
        until("the polls list it waiting", || {
            gateway.world().listed.len() >= lists + 4
        });
        // The reading held is one begun while the polls list it waiting:
        // the one under way when they first did may have begun before.
        let earlier = Arc::new(Hold::default());
        gateway.world().hold_next = Some(("slow-probe", earlier.clone()));
        until("a reading is held at its first reply", || {
            earlier.entered.load(Ordering::Acquire)
        });
        let held = Arc::new(Hold::default());
        gateway.world().hold_next = Some(("slow-probe", held.clone()));
        earlier.released.store(true, Ordering::Release);
        until("the next reading is held at its first reply", || {
            held.entered.load(Ordering::Acquire)
        });
        let lane = *reads(&gateway, SLOW).last().unwrap();
        let before = events_reads(&gateway);
        settle(&gateway, SLOW, "p-slow");
        // The card goes while that reading's reply is still held: the
        // pretend gateway holds a reply for fifteen seconds at most.
        let gone = within(Duration::from_secs(10), || !shows(&lab, "p-slow"));
        held.released.store(true, Ordering::Release);
        assert!(gone, "the lane waited for a reading given up");
        // Every read of the events since came on another socket, and the
        // one given up asks for none once its reply is let go.
        std::thread::sleep(HELD * 3);
        let since = events_reads(&gateway)[before.len()..].to_vec();
        assert!(!since.is_empty());
        assert!(
            since.iter().all(|socket| *socket != lane),
            "{since:?} on the lane {lane}"
        );
        assert!(gateway.world().calls("session.resume").is_empty());
        assert!(lab.cards().is_empty());
    });
}

/// The last look before a snapshot subscribes. A reading gets as far as
/// its snapshot, and the socket of that snapshot is held as it opens:
/// getting there takes a ticket and a dial, over a far link a moment
/// each. The session leaves the live list meanwhile, and the model is
/// seen to know it. The socket then opens, and no `session.resume` goes
/// out on it: the reading is asked whether it still counts in the step
/// that marks the visit as writing.
#[test]
fn a_snapshot_whose_session_left_the_list_while_its_socket_opened_subscribes_to_nothing() {
    within_the_limit(|| {
        let (lab, gateway) = slowed();
        let shared = lab.observer.as_ref().unwrap().local();
        // The lane has its socket open and has read on it. Every socket
        // opened from now on is held as it opens, and with the polls'
        // socket and the lane's both open, the next is the snapshot's.
        until("the lane reads the slow session", || {
            reads(&gateway, SLOW).iter().any(|socket| *socket != 1)
        });
        let sockets = gateway.world().sockets;
        let second = Arc::new(Hold::default());
        *lab.route.held.lock().unwrap() = Some(("prepare", second.clone()));
        // A request no event shows: the session waits, and only a
        // snapshot can say on what. A reading begun while the polls list
        // it waiting finds nothing waited on in the events and goes for
        // its snapshot.
        let hidden = json!({"kind":"clarify", "prompt_id":"p-hidden",
            "execution_generation":1, "question":"Synthetic?", "choices":["one"],
            "multi_select":false, "timeout_s":600});
        gateway
            .world()
            .stream_in(PROFILE, SLOW)
            .prompts
            .push(hidden);
        let reached = within(Duration::from_secs(10), || {
            second.entered.load(Ordering::Acquire)
        });
        if reached {
            gateway.world().unlisted.push(SLOW.into());
        }
        let left = reached
            && within(Duration::from_secs(10), || {
                let statuses = shared.model().gateway.statuses();
                !statuses.iter().any(|(s, _)| s.id() == SLOW)
            });
        let before = gateway.world().calls("session.resume").len();
        // Let go whatever happened, so that nothing stays held.
        second.released.store(true, Ordering::Release);
        assert!(reached, "the reading never went for its snapshot");
        assert!(left, "the model never saw the session leave the list");
        // The snapshot's socket opens, and says nothing.
        until("the snapshot's socket opens", || {
            gateway.world().sockets > sockets
        });
        until("the lane ends", || shared.apart.count() == 0);
        let world = gateway.world();
        let after = world.calls("session.resume").len();
        assert_eq!(
            (before, after),
            (0, 0),
            "a snapshot subscribed to a session off the live list"
        );
        assert_eq!(world.sockets, sockets + 1, "one socket, the snapshot's");
        let on_it = |(socket, _): &&(usize, Value)| *socket == world.sockets;
        let said: Vec<_> = world.received.iter().filter(on_it).collect();
        assert!(said.is_empty(), "{said:?}");
        drop(world);
        assert!(lab.cards().is_empty());
    });
}

/// The look before each call, by itself. A reading that counts no more
/// when its round begins sends nothing at all: here a newer reading of
/// the same session was begun, and the older is then run, on a socket
/// of its own that carries nothing else. The newer one, which counts,
/// is asked on that same socket after it.
#[test]
fn a_reading_that_counts_no_more_before_its_first_call_sends_nothing() {
    within_the_limit(|| {
        // No lane: the two readings are run by hand.
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        gateway.tells_quietly(PROFILE, SLOW, "message.start", json!({}));
        lab.follow();
        let shared = lab.observer.as_ref().unwrap().local();
        let listed = |shared: &Shared| {
            let statuses = shared.model().gateway.statuses();
            let slow = |(at, _): &(SessionRef, Status)| at.id() == SLOW;
            statuses.into_iter().find(slow).map(|(at, _)| at)
        };
        until("the polls list the session", || listed(&shared).is_some());
        let at = listed(&shared).unwrap();
        let instance = match shared.model().gateway.connection() {
            Connection::Ready(instance) => instance.clone(),
            other => panic!("not ready: {other:?}"),
        };
        let mut socket = opened(&shared, &instance).ok().expect("a socket");
        let mine = gateway.world().sockets;
        let on_mine = |gateway: &Fake| {
            let world = gateway.world();
            let frames = world.received.iter();
            frames.filter(|(socket, _)| *socket == mine).count()
        };
        assert_eq!(on_mine(&gateway), 0);

        let mut older = shared.model().gateway.poll_apart(&at).unwrap();
        let mut newer = shared.model().gateway.poll_apart(&at).unwrap();
        assert!(!shared.model().gateway.counts(&older));
        let ran = rounds(&shared, &mut socket, &instance, &mut older, false);
        assert!(matches!(ran, Ok(None)), "the older reading was run");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(on_mine(&gateway), 0, "a reading that counts no more asked");

        let ran = rounds(&shared, &mut socket, &instance, &mut newer, false);
        assert!(matches!(ran, Ok(Some(_))), "the newer reading was not run");
        assert!(on_mine(&gateway) >= 2, "the probe and the events");
        assert!(gateway.world().calls("session.resume").is_empty());
        socket.close();
    });
}

/// Whether `met` came true within `time`: a wait that says how it ended,
/// for a test that has something to let go of before it fails.
fn within(time: Duration, mut met: impl FnMut() -> bool) -> bool {
    let limit = std::time::Instant::now() + time;
    while !met() {
        if std::time::Instant::now() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// The reads of the slow session's events themselves, which name the
/// stream's epoch, with the socket each came on.
fn events_reads(gateway: &Fake) -> Vec<usize> {
    let calls = gateway.world().calls("session.events.since");
    let of = |(_, frame): &&(usize, Value)| {
        frame["params"]["session_id"] == SLOW && frame["params"].get("replay_epoch").is_some()
    };
    calls.iter().filter(of).map(|(socket, _)| *socket).collect()
}
