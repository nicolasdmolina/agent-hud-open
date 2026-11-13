//! Several readers of one source: core keeps each reading with the reader
//! that made it, so a session follows the reading that lists it, with that
//! reading's own time, and one reader never renews, replaces or lapses what
//! another told. The readers here stand for the followers of two gateways;
//! core knows them only by the names they give.
use super::*;
use crate::requests::{Requests, LIVE_STATES_MS};
use crate::SourceRoot;
use agent_hud_agents::{parse_event, LiveState, LiveStates};
use rusqlite::Connection;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, sync::Arc};

const SCHEMA: &str = include_str!("../../../agents/tests/fixtures/hermes/macos/state.schema.sql");
const SOURCE: &str = "hermes";
const NEAR: &str = "near";
const FAR: &str = "far";
const ONE: &str = "hermes:s1";
const TWO: &str = "hermes:s2";
const SECOND: i64 = 1_000;
const MINUTE: i64 = 60 * SECOND;
const DAYS: i64 = 24 * 60 * MINUTE;

/// A core reading a Hermes `state.db` with two sessions, `s1` and `s2`,
/// on a clock the test moves. Every time is given in ms from `at`.
struct Lab {
    core: AgentHudCore,
    at: i64,
    _root: tempfile::TempDir,
}

impl Lab {
    /// Both sessions started `age` ms ago and wrote nothing since.
    fn new(age: i64) -> Self {
        let at = chrono::Utc::now().timestamp_millis();
        crate::snapshot::PINNED_CLOCK.set(Some(at));
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("hermes");
        fs::create_dir_all(&folder).unwrap();
        let db = Connection::open(folder.join("state.db")).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let started = (at - age) as f64 / 1000.0;
        for id in ["s1", "s2"] {
            db.execute(
                "INSERT INTO sessions(id,source,model,started_at,last_activity_at) VALUES(?1,'cron','m',?2,?2)",
                rusqlite::params![id, started],
            )
            .unwrap();
        }
        let path = folder.join("state.db");
        let provider = "hermes";
        let roots = vec![SourceRoot { path, provider }];
        let core = AgentHudCore::new(roots, root.path().join("hud"));
        Self {
            core,
            at,
            _root: root,
        }
    }
    /// Moves the clock to `after` ms from the start.
    fn pin(&self, after: i64) {
        crate::snapshot::PINNED_CLOCK.set(Some(self.at + after));
    }
    /// A Hermes hook of `s1` that ran at `after`.
    fn hook(&mut self, event: &str, after: i64) {
        let payload = json!({"hook_event_name": event, "session_id": "s1", "cwd": "/tmp",
            "extra": {"turn_id": "s1:turn-1", "surface": "cli"}});
        let parsed = parse_event("hermes", "--completion-hook", &payload, self.at + after);
        self.core.ingest_hook_events(vec![parsed.unwrap().unwrap()]);
    }
    /// `reader` tells what it read at `after`.
    fn read(&self, reader: &str, sessions: &[(&str, LiveState)], after: i64) {
        tell(&self.core.requests(), reader, sessions, self.at + after);
    }
    /// A session's state in a fresh snapshot, or None where it is not
    /// listed.
    fn state(&mut self, id: &str) -> Option<String> {
        let core = &mut self.core;
        let mut snapshot = core.request("refresh", json!({})).unwrap();
        for _ in 0..10 {
            if core.reads.pending.is_empty() {
                break;
            }
            snapshot = core.request("snapshot", json!({})).unwrap();
        }
        let sessions = snapshot["sessions"].as_array()?;
        let row = sessions.iter().find(|row| row["id"] == id)?;
        row["state"].as_str().map(str::to_owned)
    }
    /// Counts every time core wakes the HUD from here on.
    fn wakes(&self) -> Arc<AtomicUsize> {
        counted(&self.core.requests())
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        crate::snapshot::PINNED_CLOCK.set(None);
    }
}

fn tell(requests: &Requests, reader: &str, sessions: &[(&str, LiveState)], read_ms: i64) {
    let sessions = sessions.iter().map(|(id, state)| (id.to_string(), *state));
    let sessions = sessions.collect();
    requests.set_native_states(SOURCE, reader, &LiveStates { sessions, read_ms });
}

fn counted(requests: &Requests) -> Arc<AtomicUsize> {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    requests.set_waker(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    wakes
}

/// When a reader's reading was taken, where core holds one.
fn read_ms(requests: &Requests, reader: &str) -> Option<i64> {
    let all = requests.native_states();
    all.of(SOURCE, reader).map(|reading| reading.read_ms)
}

fn waiting(requests: &Requests) -> Vec<String> {
    let all = requests.native_waiting_sessions();
    let mut sessions: Vec<String> = all.get(SOURCE).into_iter().flatten().cloned().collect();
    sessions.sort();
    sessions
}

fn some(state: &str) -> Option<String> {
    Some(state.to_owned())
}

/// The design review's case (ticket 20, finding 1): this Mac's reader
/// reads a session running, its completion hook arrives, and a far reader
/// polls afterwards. The far poll's newer time says nothing of the session.
#[test]
fn a_completion_hook_stands_when_another_reader_reads_other_sessions_later() {
    let mut lab = Lab::new(10 * MINUTE);
    lab.hook("pre_tool_call", -3 * MINUTE);
    assert_eq!(lab.state(ONE), some("idle"), "the hooks' own guess");
    lab.read(NEAR, &[(ONE, LiveState::Running)], -3 * SECOND);
    assert_eq!(lab.state(ONE), some("running"));
    // The other reader reports its own sessions: the first keeps its
    // reader's reading.
    lab.read(FAR, &[(TWO, LiveState::Running)], -2_500);
    assert_eq!(lab.state(ONE), some("running"));
    // The turn ends: the hook is newer than the reading that lists it.
    lab.hook("on_session_end", -2 * SECOND);
    assert_eq!(lab.state(ONE), some("idle"));
    // The other reader reads after the hook, twice: no news of this one.
    lab.read(FAR, &[(TWO, LiveState::Running)], -SECOND);
    assert_eq!(lab.state(ONE), some("idle"), "it follows its hook");
    lab.read(FAR, &[(TWO, LiveState::Running)], 0);
    assert_eq!(lab.state(ONE), some("idle"), "it follows its hook");
    assert_eq!(lab.state(TWO), some("running"));
    // Its own reader reads again after the hook, and stands again.
    lab.read(NEAR, &[(ONE, LiveState::Running)], 0);
    assert_eq!(lab.state(ONE), some("running"));
}

#[test]
fn readers_of_different_ages_each_lapse_thirty_seconds_after_their_own_reading() {
    let mut lab = Lab::new(3 * DAYS);
    let requests = lab.core.requests();
    assert_eq!(lab.state(ONE), None, "three days quiet, neither is listed");
    assert_eq!(lab.state(TWO), None);
    lab.read(NEAR, &[(ONE, LiveState::Running)], -20 * SECOND);
    lab.read(FAR, &[(TWO, LiveState::Waiting)], -5 * SECOND);
    assert_eq!(lab.state(ONE), some("running"));
    assert_eq!(lab.state(TWO), some("waiting"));
    let near_lapses = -20 * SECOND + LIVE_STATES_MS;
    let far_lapses = -5 * SECOND + LIVE_STATES_MS;
    assert_eq!(requests.next_deadline(), Some(lab.at + near_lapses));

    lab.pin(near_lapses - 1);
    assert_eq!(lab.state(ONE), some("running"), "a moment before it lapses");
    lab.pin(near_lapses);
    assert_eq!(lab.state(ONE), None, "the older reading lapsed");
    assert_eq!(lab.state(TWO), some("waiting"), "the newer one has not");
    assert_eq!(read_ms(&requests, NEAR), None);
    assert_eq!(read_ms(&requests, FAR), Some(lab.at - 5 * SECOND));
    assert_eq!(requests.next_deadline(), Some(lab.at + far_lapses));

    lab.pin(far_lapses - 1);
    assert_eq!(lab.state(TWO), some("waiting"), "a moment before it lapses");
    lab.pin(far_lapses);
    assert_eq!(lab.state(TWO), None, "lapsed by its own time");
    assert!(requests.native_states().is_empty());
}

#[test]
fn a_reader_that_stops_lapses_alone_while_another_reads_every_second() {
    let mut lab = Lab::new(3 * DAYS);
    let requests = lab.core.requests();
    let wakes = lab.wakes();
    // One reader reads once and then says nothing more. The other reads
    // every second for a minute.
    lab.read(NEAR, &[(ONE, LiveState::Running)], 0);
    let lapses = lab.at + LIVE_STATES_MS;
    for second in 1..=60 {
        let after = second * SECOND;
        lab.pin(after);
        lab.read(FAR, &[(TWO, LiveState::Running)], after);
        if second < 30 {
            // What core asks to be woken for is still the silent reader.
            assert_eq!(requests.next_deadline(), Some(lapses));
            assert_eq!(lab.state(ONE), some("running"), "at {second} s");
        }
        if second == 30 {
            // Core is woken at the time it gave, with no call from either
            // reader, and tells the HUD.
            let revision = requests.revision();
            requests.expire(lapses - 1);
            assert_eq!(requests.revision(), revision, "still believed");
            let before = wakes.load(Ordering::SeqCst);
            requests.expire(lapses);
            assert_eq!(wakes.load(Ordering::SeqCst), before + 1);
            assert_eq!(read_ms(&requests, NEAR), None);
        }
        if second >= 30 {
            assert_eq!(lab.state(ONE), None, "lapsed, at {second} s");
        }
        assert_eq!(lab.state(TWO), some("running"), "at {second} s");
        assert_eq!(read_ms(&requests, FAR), Some(lab.at + after));
    }
    let renewed = lab.at + 60 * SECOND + LIVE_STATES_MS;
    assert_eq!(requests.next_deadline(), Some(renewed));
}

#[test]
fn two_readers_listing_one_session_the_newer_reading_wins_in_either_order() {
    // Told in the order they were read, and the other way round.
    for older_first in [true, false] {
        let mut lab = Lab::new(10 * MINUTE);
        let older = (NEAR, [(ONE, LiveState::Running)], -2 * SECOND);
        let newer = (FAR, [(ONE, LiveState::Waiting)], -SECOND);
        let order = if older_first {
            [older, newer]
        } else {
            [newer, older]
        };
        for (reader, sessions, after) in order {
            lab.read(reader, &sessions, after);
        }
        assert_eq!(
            lab.state(ONE),
            some("waiting"),
            "older first: {older_first}"
        );
        // A hook between the two readings is older news than the newer.
        lab.hook("on_session_end", -1_500);
        assert_eq!(
            lab.state(ONE),
            some("waiting"),
            "older first: {older_first}"
        );
        // The newer reader ends: the session is back with the reading
        // still held, which its hook has passed.
        lab.core.requests().end_native_reader(SOURCE, FAR);
        assert_eq!(lab.state(ONE), some("idle"), "older first: {older_first}");
    }
    // Two readings of one moment: the same answer whichever came first.
    let told = |first: &str, second: &str| {
        let requests = Requests::default();
        let state = |reader: &str| match reader {
            NEAR => LiveState::Running,
            _ => LiveState::Idle,
        };
        tell(&requests, first, &[(ONE, state(first))], 1_000);
        tell(&requests, second, &[(ONE, state(second))], 1_000);
        requests.native_states().told(SOURCE, ONE, 1_000)
    };
    assert_eq!(told(NEAR, FAR), told(FAR, NEAR));
    assert!(told(NEAR, FAR).is_some());
}

#[test]
fn one_reader_replacing_its_waiting_list_leaves_anothers_flags() {
    let requests = Requests::default();
    let list = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    requests.set_native_waiting(SOURCE, NEAR, &list(&[ONE]), 0);
    requests.set_native_waiting(SOURCE, FAR, &list(&[TWO, "hermes:s3"]), 0);
    assert_eq!(
        waiting(&requests),
        list(&["hermes:s1", "hermes:s2", "hermes:s3"])
    );
    // One reader's whole new list, then its empty one.
    requests.set_native_waiting(SOURCE, FAR, &list(&["hermes:s3"]), 0);
    assert_eq!(waiting(&requests), list(&["hermes:s1", "hermes:s3"]));
    requests.set_native_waiting(SOURCE, FAR, &[], 0);
    assert_eq!(waiting(&requests), list(&["hermes:s1"]));
    // Telling a list again changes nothing and wakes nothing.
    let (revision, wakes) = (requests.revision(), counted(&requests));
    requests.set_native_waiting(SOURCE, NEAR, &list(&[ONE]), 0);
    requests.set_native_waiting(SOURCE, FAR, &[], 0);
    assert_eq!(requests.revision(), revision);
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    // A session both readers hold a card in waits until both let go.
    requests.set_native_waiting(SOURCE, FAR, &list(&[ONE]), 0);
    requests.set_native_waiting(SOURCE, NEAR, &[], 0);
    assert_eq!(waiting(&requests), list(&["hermes:s1"]));
    requests.set_native_waiting(SOURCE, FAR, &[], 0);
    assert!(requests.native_waiting_sessions().is_empty());
}

#[test]
fn ending_a_reader_clears_its_states_and_flags_and_no_one_elses() {
    let requests = Requests::default();
    tell(&requests, NEAR, &[(ONE, LiveState::Running)], 1_000);
    tell(&requests, FAR, &[(TWO, LiveState::Running)], 2_000);
    requests.set_native_waiting(SOURCE, NEAR, &[ONE.to_owned()], 0);
    requests.set_native_waiting(SOURCE, FAR, &[TWO.to_owned()], 0);
    // The same reader name under another source is another reader.
    let sessions = vec![("other:s".to_owned(), LiveState::Idle)];
    let reading = LiveStates {
        sessions,
        read_ms: 2_000,
    };
    requests.set_native_states("other", FAR, &reading);

    let (revision, wakes) = (requests.revision(), counted(&requests));
    requests.end_native_reader(SOURCE, FAR);
    assert_ne!(requests.revision(), revision);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(read_ms(&requests, FAR), None);
    assert_eq!(read_ms(&requests, NEAR), Some(1_000));
    assert_eq!(waiting(&requests), vec![ONE.to_owned()]);
    let all = requests.native_states();
    assert!(all.of("other", FAR).is_some(), "another source's reader");
    assert_eq!(all.told(SOURCE, TWO, 2_000), None);
    assert_eq!(
        all.told(SOURCE, ONE, 2_000),
        Some((LiveState::Running, 1_000))
    );
    // Ending it again, or a reader that never told anything, is nothing.
    let revision = requests.revision();
    requests.end_native_reader(SOURCE, FAR);
    requests.end_native_reader(SOURCE, "never");
    assert_eq!(requests.revision(), revision);
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_reader_renews_and_replaces_only_its_own_reading() {
    let requests = Requests::default();
    tell(&requests, NEAR, &[(ONE, LiveState::Running)], 1_000);
    tell(&requests, FAR, &[(TWO, LiveState::Running)], 5_000);
    // The other reader's newer readings leave the first one's time alone.
    tell(&requests, FAR, &[(TWO, LiveState::Running)], 9_000);
    assert_eq!(read_ms(&requests, NEAR), Some(1_000));
    assert_eq!(read_ms(&requests, FAR), Some(9_000));
    // The very same reading told again wakes nothing, whoever else reads.
    let (revision, wakes) = (requests.revision(), counted(&requests));
    tell(&requests, NEAR, &[(ONE, LiveState::Running)], 1_000);
    tell(&requests, FAR, &[(TWO, LiveState::Running)], 9_000);
    assert_eq!(requests.revision(), revision);
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    // A reader that lists nothing any more drops its own reading only.
    tell(&requests, FAR, &[], 10_000);
    assert_eq!(read_ms(&requests, FAR), None);
    assert_eq!(read_ms(&requests, NEAR), Some(1_000));
    assert_eq!(requests.next_deadline(), Some(1_000 + LIVE_STATES_MS));
}
