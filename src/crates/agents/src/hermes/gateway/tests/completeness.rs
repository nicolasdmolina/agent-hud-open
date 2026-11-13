//! Whether a poll finds everything that waits in a session whose stream
//! Hermes has dropped events of. It reads from the oldest event Hermes
//! still keeps, so a request whose event is kept is always seen. A request
//! whose event was dropped is found by one snapshot for each stretch of
//! waiting, when a card of the session closes, after the socket was lost,
//! and every 30 s while the session goes on waiting. The cases of the third
//! and fourth reviews (`evidence/12/verifier-report-3.md` and `-4.md`) are
//! here under their own names.
use super::super::*;
use super::fake::{identified, names, poll, Hermes};
use serde_json::{json, Value};

const S: &str = "local-long";

fn work() -> Profile {
    Profile::named("verify")
}

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new("local"));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

/// Events that ask and settle nothing, each of about `bytes`.
fn noise(hermes: &mut Hermes, count: usize, bytes: usize) {
    let body = json!({ "synthetic": "x".repeat(bytes) });
    for _ in 0..count {
        hermes.publish(&work(), S, "session.info", 1, body.clone());
    }
    hermes.keep_what_hermes_keeps(&work(), S);
}

fn asks(hermes: &mut Hermes, id: &str, kind: &str) {
    let body = match kind {
        "approval" => json!({"kind": kind, "prompt_id": id, "execution_generation": 1,
            "command": "synthetic command", "description": "synthetic approval",
            "choices": ["once", "deny", "session", "always"], "timeout_s": 20}),
        _ => json!({"kind": kind, "prompt_id": id, "execution_generation": 1,
            "question": "Synthetic question", "choices": [], "timeout_s": 20}),
    };
    let event = if kind == "approval" {
        "approval.request"
    } else {
        "clarify.request"
    };
    hermes.publish(&work(), S, event, 1, body);
    hermes.keep_what_hermes_keeps(&work(), S);
}

fn settles(hermes: &mut Hermes, id: &str) {
    let body: Value = json!({"prompt_id": id, "execution_generation": 1});
    hermes.publish(&work(), S, "clarify.settled", 1, body);
    hermes.keep_what_hermes_keeps(&work(), S);
}

fn kept(hermes: &mut Hermes) -> (u64, u64) {
    let stream = hermes.stream(&work(), S);
    (stream.dropped, stream.latest)
}

#[test]
fn retained_older_request_must_not_be_hidden_by_newer_prompt() {
    let (mut g, mut h) = start();
    noise(&mut h, 568, 0);
    asks(&mut h, "older", "approval"); // 569, the oldest event Hermes keeps
    noise(&mut h, 63, 0);
    asks(&mut h, "newer", "approval"); // 633
    noise(&mut h, 447, 0);
    assert_eq!(kept(&mut h), (568, 1080));
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["newer", "older"]);
    // One read, from exactly the last event dropped: everything kept.
    assert_eq!(h.read_from(), [568]);
}

#[test]
fn byte_cap_retry_must_not_hide_retained_request() {
    let (mut g, mut h) = start();
    noise(&mut h, 99, 20_000);
    asks(&mut h, "older", "approval"); // 100
    noise(&mut h, 149, 20_000);
    asks(&mut h, "newer", "approval"); // 250
    noise(&mut h, 50, 20_000);
    // Hermes dropped by size, well before 512 events.
    let (dropped, latest) = kept(&mut h);
    assert!(latest == 300 && dropped > 0 && dropped < 100, "{dropped}");
    poll(&mut g, &mut h, 0);
    assert_eq!(names(&g), ["newer", "older"]);
    // The search ends on the last event dropped, in a bounded number of calls.
    let read = h.read_from();
    assert_eq!(read.iter().min(), Some(&0));
    assert!(
        read.contains(&dropped) && !read.contains(&(dropped + 1)),
        "{read:?}"
    );
    assert!(h.asked.len() - 2 <= 12, "{read:?}");
}

#[test]
fn evicted_older_request_must_trigger_complete_snapshot_despite_newer_prompt() {
    let (mut g, mut h) = start();
    asks(&mut h, "older", "approval");
    noise(&mut h, 600, 0);
    asks(&mut h, "newer", "approval");
    assert!(kept(&mut h).0 >= 1);
    // The newer one is in the events and the older one is not: the first
    // poll that sees the session wait takes a snapshot all the same.
    poll(&mut g, &mut h, 0);
    assert_eq!(
        (h.snapshots(), names(&g)),
        (1, vec!["newer".to_owned(), "older".to_owned()])
    );
}

#[test]
fn evicted_showable_request_must_not_be_hidden_by_unshowable_prompt() {
    let (mut g, mut h) = start();
    asks(&mut h, "older", "clarify");
    noise(&mut h, 600, 0);
    asks(&mut h, "newer", "future-kind");
    poll(&mut g, &mut h, 0);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["older".to_owned()]));
}

/// The third review asserted no snapshot at all here. That cannot hold
/// together with the case above it: with events dropped, the events kept
/// never tell whether an older request waits, so the first poll of a
/// stretch of waiting takes one snapshot. What it must not do is take one
/// at every poll, which is what the review found (three in three polls).
#[test]
fn retained_single_request_must_not_force_snapshot_every_poll() {
    let (mut g, mut h) = start();
    noise(&mut h, 568, 0);
    asks(&mut h, "older", "clarify"); // 569, the oldest event Hermes keeps
    noise(&mut h, 511, 0);
    assert_eq!(kept(&mut h), (568, 1080));
    let mut snapshots = Vec::new();
    for now in [0, 1_000, 2_000, 3_000] {
        poll(&mut g, &mut h, now);
        snapshots.push(h.snapshots());
        assert_eq!(names(&g), ["older"]);
    }
    assert_eq!(snapshots, [1, 0, 0, 0]);
}

#[test]
fn a_stream_that_has_dropped_nothing_needs_no_snapshot() {
    let (mut g, mut h) = start();
    noise(&mut h, 400, 0);
    asks(&mut h, "p1", "clarify");
    noise(&mut h, 100, 0);
    assert_eq!(kept(&mut h), (0, 501));
    for now in [0, 1_000] {
        poll(&mut g, &mut h, now);
        assert_eq!(
            (h.snapshots(), h.read_from(), names(&g)),
            (0, vec![0], vec!["p1".to_owned()])
        );
    }
}

#[test]
fn each_stretch_of_waiting_gets_its_own_snapshot() {
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "p1", "clarify");
    poll(&mut g, &mut h, 0);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["p1".to_owned()]));
    // A second request in the same stretch is in the events: no snapshot.
    asks(&mut h, "p2", "clarify");
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), names(&g).len()), (0, 2));
    // Both settle and the session works on: the stretch is over.
    settles(&mut h, "p1");
    settles(&mut h, "p2");
    h.stream(&work(), S).running = true;
    poll(&mut g, &mut h, 2_000);
    assert_eq!((h.snapshots(), names(&g).len()), (0, 0));
    // It waits again: one snapshot again, and none after.
    asks(&mut h, "p3", "clarify");
    poll(&mut g, &mut h, 3_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["p3".to_owned()]));
    poll(&mut g, &mut h, 4_000);
    assert_eq!(h.snapshots(), 0);
    // A restart forgets it: the new instance's first waiting poll takes one.
    identified(&mut g, "two");
    poll(&mut g, &mut h, 5_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["p3".to_owned()]));
}

#[test]
fn a_snapshot_that_failed_does_not_count_for_the_stretch() {
    let (mut g, mut h) = start();
    asks(&mut h, "older", "approval");
    noise(&mut h, 600, 0);
    asks(&mut h, "newer", "approval");
    h.failing.push(Step::Snapshot(g.session(&work(), S)));
    // The session could not be read whole this time: no card is opened on
    // half of what waits, and the next poll asks again.
    assert_eq!(poll(&mut g, &mut h, 0), []);
    poll(&mut g, &mut h, 1_000);
    assert_eq!(
        (h.snapshots(), names(&g)),
        (1, vec!["newer".to_owned(), "older".to_owned()])
    );
}

#[test]
fn a_request_dropped_while_it_shows_keeps_its_card_between_snapshots() {
    let (mut g, mut h) = start();
    asks(&mut h, "older", "clarify");
    poll(&mut g, &mut h, 0);
    assert_eq!(h.snapshots(), 0);
    // Its event is dropped while its card shows: the events no longer
    // account for the card, so a snapshot says whether it still waits.
    // The next one comes 30 s on, counted from the poll before, and the
    // card stays in between.
    noise(&mut h, 600, 0);
    let mut snapshots = Vec::new();
    for now in [1_000, 2_000, 16_000, 31_000, 32_000, 33_000] {
        assert_eq!(poll(&mut g, &mut h, now), []);
        snapshots.push(h.snapshots());
        assert_eq!(names(&g), ["older"]);
    }
    assert_eq!(snapshots, [1, 0, 0, 0, 1, 0]);
    // Hermes says the session waits no more: the card goes at that poll.
    settles(&mut h, "older");
    noise(&mut h, 600, 0);
    h.stream(&work(), S).running = true;
    poll(&mut g, &mut h, 34_000);
    assert_eq!((h.snapshots(), names(&g).len()), (0, 0));
}

// ---- The fourth review (`evidence/12/verifier-report-4.md`,
// `verifier-4-adversarial.rs`), under its own names ----

fn responded(gateway: &mut Gateway, now_ms: i64) {
    let request = gateway.waiting().next().unwrap().id();
    let reply = json!({"result": {"status": "resolved"}});
    let (request, reply) = (&request, &reply);
    gateway.apply(Input::Responded { request, reply }, now_ms);
}

#[test]
fn round4_missed_wait_transition_must_snapshot_second_episode() {
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    assert_eq!(h.snapshots(), 1);
    // Answered from its card. Hermes stops waiting, and no poll sees that.
    settles(&mut h, "first");
    responded(&mut g, 1);
    assert!(names(&g).is_empty());
    // It waits again, on a request whose event Hermes then drops with the
    // whole ring (it keeps 64 streams and 64 MiB in all).
    asks(&mut h, "second", "clarify");
    let latest = kept(&mut h).1;
    h.drop_through(&work(), S, latest);
    noise(&mut h, 1, 0);
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["second".to_owned()]));
}

#[test]
fn round4_same_instance_reconnect_must_refresh_unknown_wait_episode() {
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    // The socket drops for ten seconds, in which the first request
    // settles elsewhere and a second is asked and dropped from the stream.
    g.apply(Input::Lost(Failure::Closed(1006)), 2);
    settles(&mut h, "first");
    asks(&mut h, "second", "clarify");
    noise(&mut h, 600, 0);
    identified(&mut g, "one");
    poll(&mut g, &mut h, 10_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["second".to_owned()]));
}

#[test]
fn round4_subsequent_cache_eviction_needs_no_large_local_burst_to_hide_request() {
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    // One stretch of waiting throughout. A second request is asked, the
    // ring is dropped, and the first one settles elsewhere.
    asks(&mut h, "second", "clarify");
    let latest = kept(&mut h).1;
    h.drop_through(&work(), S, latest);
    settles(&mut h, "first");
    // The settlement accounts for the only card. It closes, and a card of
    // the session closing is what makes the next poll take a snapshot.
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), names(&g).len()), (0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["second".to_owned()]));
}

#[test]
fn a_request_no_read_can_show_is_found_by_the_next_snapshot() {
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    // The worst case: one stretch of waiting, no card closes, the socket
    // holds, and the new request's event is dropped before any poll.
    asks(&mut h, "second", "clarify");
    let latest = kept(&mut h).1;
    h.drop_through(&work(), S, latest);
    let mut found = None;
    for now in (1_000..=40_000).step_by(1_000) {
        poll(&mut g, &mut h, now);
        if names(&g).len() == 2 {
            found = Some(now);
            break;
        }
        assert_eq!((h.snapshots(), names(&g)), (0, vec!["first".to_owned()]));
    }
    // 30 s after the last snapshot, counted from the poll before: 31 s.
    assert_eq!(found, Some(SNAPSHOT_EVERY_MS + 1_000));
}

/// One poll of a stream that keeps moving under the search for its oldest
/// event, as the fourth review moved it: an event before each of three
/// reads, 512 before each of the next two, and before the next an older
/// approval, 510 events and the settlement of `first`. Then it stands
/// still. Returns the poll and how many reads it made.
fn poll_a_moving_stream(gateway: &mut Gateway, hermes: &mut Hermes) -> (Poll, usize) {
    hermes.asked.clear();
    let mut poll = gateway.poll().unwrap();
    let mut reads = 0;
    while !poll.steps().is_empty() {
        for step in poll.steps() {
            if matches!(step, Step::Since { .. }) {
                reads += 1;
                match reads {
                    2..=4 => noise(hermes, 1, 0),
                    5..=6 => noise(hermes, 512, 0),
                    7 => {
                        asks(hermes, "older", "approval");
                        noise(hermes, 510, 0);
                        settles(hermes, "first");
                    }
                    _ => {}
                }
            }
            match hermes.answer(&step) {
                Some(result) => poll.answer(&step, &result),
                None => poll.failed(&step),
            }
        }
    }
    (poll, reads)
}

#[test]
fn round4_moving_ring_must_not_discard_a_retained_request_when_budget_ends() {
    // More than 30 s after the session's last snapshot: the search runs
    // out of calls one short of the oldest event kept, which is the older
    // request. The poll knows it did not read everything and takes a
    // snapshot, so the request shows in that same poll.
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    assert_eq!(poll(&mut g, &mut h, SNAPSHOT_EVERY_MS), []);
    let (moving, reads) = poll_a_moving_stream(&mut g, &mut h);
    assert_eq!((reads, h.snapshots()), (16, 1));
    g.apply(Input::Polled(moving), SNAPSHOT_EVERY_MS + 1_000);
    assert_eq!(names(&g), ["older"]);

    // Within 30 s of the last snapshot none is taken. The poll then
    // closes nothing on what it read, though the events it did read hold
    // the settlement of `first`: it stays, and the next poll, of a stream
    // that stands still, reads everything.
    let (mut g, mut h) = start();
    noise(&mut h, 600, 0);
    asks(&mut h, "first", "clarify");
    poll(&mut g, &mut h, 0);
    let (moving, reads) = poll_a_moving_stream(&mut g, &mut h);
    assert_eq!((reads, h.snapshots()), (16, 0));
    assert_eq!(g.apply(Input::Polled(moving), 1_000), []);
    assert_eq!(names(&g), ["first"]);
    poll(&mut g, &mut h, 2_000);
    assert_eq!((h.snapshots(), names(&g)), (0, vec!["older".to_owned()]));
}

#[test]
fn a_stream_of_which_nothing_is_kept_is_found_out_in_three_calls() {
    // Two lists, three reads and the snapshot make the six steps below.
    let (mut g, mut h) = start();
    asks(&mut h, "p1", "clarify");
    noise(&mut h, 700, 0);
    // Hermes dropped the whole ring (it keeps 64 streams and 64 MiB in all).
    h.drop_through(&work(), S, 701);
    poll(&mut g, &mut h, 0);
    // Where the stream is, then two reads: where the oldest event would be
    // by the count alone, and whether the very last event is kept.
    assert_eq!((h.asked.len(), h.read_from()), (6, vec![189, 700]));
    assert_eq!((h.snapshots(), names(&g)), (1, vec!["p1".to_owned()]));
    // A session that only works is never subscribed to, whatever was dropped.
    settles(&mut h, "p1");
    h.stream(&work(), S).running = true;
    h.drop_through(&work(), S, 702);
    poll(&mut g, &mut h, 1_000);
    assert_eq!((h.snapshots(), names(&g).len()), (0, 0));
}
