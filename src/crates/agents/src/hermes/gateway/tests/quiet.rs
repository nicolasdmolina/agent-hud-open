//! Sessions nothing has changed in for a long while (`quiet.rs`, ticket
//! 67): a session Hermes lists as working for days, whose stream stands
//! still, stops hurrying the polls and the history, is still read at
//! every poll, and counts again the moment it moves.
use super::super::*;
use super::fake::{ask, identified, names, poll, Hermes};
use super::{far, local, INSTALL};
use serde_json::json;

const STUCK: &str = "stuck";
const MINUTE: i64 = 60_000;

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

/// Hermes lists `session` as working whatever its stream says.
fn listed_as_working(hermes: &mut Hermes, session: &str) {
    hermes.stream(&Profile::Default, session).status = Some("working".into());
}

/// The paces as `(poll_ms, lively, working)`: all three quick, or none.
fn paces(gateway: &Gateway) -> (i64, bool, bool) {
    (gateway.poll_ms(), gateway.lively(), gateway.working())
}
const QUICK: (i64, bool, bool) = (BUSY_POLL_MS, true, true);
const SLOW: (i64, bool, bool) = (IDLE_POLL_MS, false, false);

fn question(id: &str) -> serde_json::Value {
    json!({"kind": "clarify", "prompt_id": id, "execution_generation": 1,
        "question": "Synthetic question", "choices": [], "timeout_s": 20})
}

/// The reads of `session`'s events the last poll asked.
fn reads(hermes: &Hermes, session: &str) -> usize {
    let read = |step: &&Step| matches!(step, Step::Since { session: at, .. } if at.id() == session);
    hermes.asked.iter().filter(read).count()
}

#[test]
fn the_quiet_time_is_hermes_own_limit_for_a_turn_in_which_nothing_happens() {
    // `agent.gateway_timeout`, 1800 s (`hermes_cli/config_defaults.py`).
    assert_eq!(QUIET_MS, 1_800_000);
}

#[test]
fn a_session_listed_as_working_whose_stream_stands_still_stops_counting_as_work() {
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 1_000);
    assert_eq!(paces(&g), QUICK, "first listed: it counts at once");
    // Listed as working at every poll, its stream where it was.
    poll(&mut g, &mut h, 1_000 + QUIET_MS - 1);
    assert_eq!(paces(&g), QUICK, "a moment before the quiet time");
    poll(&mut g, &mut h, 1_000 + QUIET_MS);
    assert_eq!(paces(&g), SLOW, "quiet: neither pace is hurried");
    // Its row says what Hermes says, and it is read as it was before.
    let at = g.session(&Profile::Default, STUCK).unwrap();
    assert_eq!(g.status(&at), Some(Status::Working));
    let live = g.live_states();
    assert_eq!(live.sessions, [(at.hud_id(), crate::LiveState::Running)]);
    assert!(reads(&h, STUCK) > 0, "a quiet session is still read");
    for days in 1..=6 {
        poll(&mut g, &mut h, days * 24 * 60 * MINUTE);
        assert_eq!(paces(&g), SLOW, "day {days}");
        assert!(reads(&h, STUCK) > 0, "day {days}");
    }
}

#[test]
fn a_quiet_session_that_moves_again_counts_at_once() {
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    // One event on its stream: the same epoch, the next place.
    h.publish(&Profile::Default, STUCK, "tool.start", 1, json!({}));
    poll(&mut g, &mut h, QUIET_MS + 10_000);
    assert_eq!(paces(&g), QUICK, "its stream moved");
    // And it is quiet again only a whole quiet time after that.
    poll(&mut g, &mut h, 2 * QUIET_MS + 9_999);
    assert_eq!(paces(&g), QUICK);
    poll(&mut g, &mut h, 2 * QUIET_MS + 10_000);
    assert_eq!(paces(&g), SLOW);
    // Another stream of the same session (the gateway replaced it) is a
    // move too, though its latest event is no later.
    h.stream(&Profile::Default, STUCK).epoch = "epoch-2".into();
    poll(&mut g, &mut h, 3 * QUIET_MS);
    assert_eq!(paces(&g), QUICK, "another epoch");
    // So is the list saying something else of it, with its stream still.
    poll(&mut g, &mut h, 5 * QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    h.stream(&Profile::Default, STUCK).status = Some("idle".into());
    poll(&mut g, &mut h, 5 * QUIET_MS + 1_000);
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 5 * QUIET_MS + 2_000);
    assert_eq!(paces(&g), QUICK, "listed as something else, and back");
}

#[test]
fn a_request_asked_in_a_quiet_session_opens_its_card_at_the_next_poll() {
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    h.stream(&Profile::Default, STUCK).status = None;
    h.publish(&Profile::Default, STUCK, "message.start", 1, json!({}));
    h.publish(
        &Profile::Default,
        STUCK,
        "clarify.request",
        1,
        question("p1"),
    );
    // The one poll that comes at the slow pace finds it.
    let effects = poll(&mut g, &mut h, QUIET_MS + IDLE_POLL_MS);
    let opened = |effect: &&Effect| matches!(effect, Effect::Open(_));
    assert_eq!(effects.iter().filter(opened).count(), 1);
    assert_eq!(names(&g), ["p1"]);
    assert_eq!(g.poll_ms(), BUSY_POLL_MS, "a card that shows hurries");
    assert!(g.lively());
    // A card keeps the polls quick for as long as it shows, also once
    // nothing has moved in its session for the quiet time.
    poll(&mut g, &mut h, 3 * QUIET_MS);
    assert_eq!(names(&g), ["p1"]);
    assert_eq!((g.poll_ms(), g.lively()), (BUSY_POLL_MS, true));
}

#[test]
fn a_read_that_fails_is_no_movement_and_no_read_at_all_goes_quiet_by_the_list() {
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    let at = g.session(&Profile::Default, STUCK).unwrap();
    let (session, cursor) = (at.clone(), None);
    let first = Step::Since { session, cursor };
    // Its stream is never read: every read of it fails.
    for now in [0, QUIET_MS - 1] {
        h.failing.push(first.clone());
        poll(&mut g, &mut h, now);
        assert_eq!(paces(&g), QUICK, "at {now}");
    }
    h.failing.push(first.clone());
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW, "listed the same for the quiet time");
    // A stream that was read, then is not, then is where it was: still.
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    h.failing.push(first.clone());
    poll(&mut g, &mut h, QUIET_MS - 1_000);
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW, "a failed read between two that agree");
}

#[test]
fn a_session_counts_from_when_it_is_first_listed_and_anew_after_a_restart() {
    let (mut g, mut h) = start();
    poll(&mut g, &mut h, 0);
    // Listed long after the gateway was first polled: it counts, also
    // where the first read of its stream fails.
    listed_as_working(&mut h, STUCK);
    let session = g.session(&Profile::Default, STUCK).unwrap();
    let cursor = None;
    h.failing.push(Step::Since { session, cursor });
    poll(&mut g, &mut h, 4 * QUIET_MS);
    assert_eq!(paces(&g), QUICK);
    // The read that first says where its stream is tells of no move.
    assert!(h.failing.is_empty());
    poll(&mut g, &mut h, 5 * QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    // It leaves the list and is listed again: seen for the first time.
    h.streams.clear();
    poll(&mut g, &mut h, 5 * QUIET_MS + 1_000);
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 5 * QUIET_MS + 2_000);
    assert_eq!(paces(&g), QUICK, "listed again");
    // The gateway restarts and lists it as before: nothing is known of
    // the instance that is gone.
    poll(&mut g, &mut h, 7 * QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    g.apply(Input::Restarted, 7 * QUIET_MS + 1_000);
    identified(&mut g, "two");
    poll(&mut g, &mut h, 7 * QUIET_MS + 2_000);
    assert_eq!(paces(&g), QUICK, "another instance");
}

#[test]
fn a_session_that_is_read_apart_moves_by_its_own_reading() {
    let (mut g, mut h) = start();
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    // Slow to read, it is read apart from here on: each poll is told
    // its read failed (`apart.rs`).
    let at = g.session(&Profile::Default, STUCK).unwrap();
    let beside = |g: &mut Gateway, h: &mut Hermes, now: i64| {
        let (session, cursor) = (at.clone(), None);
        h.failing.push(Step::Since { session, cursor });
        poll(g, h, now);
    };
    beside(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    // Its stream moves, which no poll reads.
    h.publish(&Profile::Default, STUCK, "tool.start", 1, json!({}));
    beside(&mut g, &mut h, QUIET_MS + 10_000);
    assert_eq!(paces(&g), SLOW, "the polls do not read it");
    // The reading of it apart does.
    let mut reading = g.poll_apart(&at).expect("a reading apart");
    ask(&mut reading, &mut h);
    g.apply(Input::Polled(reading), QUIET_MS + 11_000);
    assert_eq!(paces(&g), QUICK, "its own reading found it moved");
}

#[test]
fn a_session_that_only_starts_for_the_quiet_time_is_quiet_too() {
    let (mut g, mut h) = start();
    h.stream(&Profile::Default, STUCK).status = Some("starting".into());
    poll(&mut g, &mut h, 0);
    assert_eq!(paces(&g), QUICK);
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW);
    // Its agent begins: it works.
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, QUIET_MS + 1_000);
    assert_eq!(paces(&g), QUICK);
}

#[test]
fn a_far_gateway_keeps_its_close_watch_only_while_a_listed_session_moved_lately() {
    let (mut g, mut h) = start();
    // Old chats a gateway lists for as long as it runs, and one session
    // it lists as working for days.
    for id in ["old-1", "old-2", "old-3"] {
        h.stream(&Profile::Default, id);
    }
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    assert!(g.lively());
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!(paces(&g), SLOW, "nothing moved in any of them");
    // A turn begins in one of the idle ones, and a look finds its
    // stream moved before the list says anything: the watch is back.
    // Not where a run of every test has the looks off for every gateway
    // (`turns.rs`, `positions_decide_no_card`): an idle session's
    // stream is then never looked at.
    let looks_off = std::env::var(super::super::turns::LOOKS).as_deref() == Ok("off");
    h.publish(&Profile::Default, "old-2", "message.start", 1, json!({}));
    h.stream(&Profile::Default, "old-2").status = Some("idle".into());
    poll(&mut g, &mut h, QUIET_MS + 10_000);
    assert_eq!(g.lively(), !looks_off, "an idle session's stream moved");
    assert_eq!(g.poll_ms(), IDLE_POLL_MS, "and none of them is busy");
    assert!(!g.working());
    // The quiet time can be set, as the lab's tests do.
    let (mut g, mut h) = start();
    g.quiet_after(5_000);
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    poll(&mut g, &mut h, 4_999);
    assert_eq!(paces(&g), QUICK);
    poll(&mut g, &mut h, 5_000);
    assert_eq!(paces(&g), SLOW);
}

#[test]
fn a_gateway_of_this_machine_keeps_its_close_watch_while_any_session_is_listed() {
    let mut g = local();
    identified(&mut g, "one");
    let mut h = Hermes::default();
    h.stream(&Profile::Default, "old-1");
    listed_as_working(&mut h, STUCK);
    poll(&mut g, &mut h, 0);
    assert_eq!((g.poll_ms(), g.lively()), (BUSY_POLL_MS, true));
    // Quiet: the model's own wait is the long one, and the watch stays,
    // as ticket 13 set it for this machine.
    poll(&mut g, &mut h, QUIET_MS);
    assert_eq!((g.poll_ms(), g.lively()), (IDLE_POLL_MS, true));
    assert!(!g.working());
    // Nothing listed: no watch.
    h.streams.clear();
    poll(&mut g, &mut h, QUIET_MS + 1_000);
    assert!(!g.lively());
}
