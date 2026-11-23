//! The sessions that are read apart from the polls. A session whose
//! events take long to read (a long stream over a slow link, a gateway
//! busy with that session) would hold up every poll that reads it, and
//! so every other session's cards. Once a poll has found a read that
//! slow (`polls.rs`), the session gets a lane: a thread and a socket of
//! its own, on which it is read by itself, again and again, for as long
//! as the polls would read it, and no more often than they come: on a
//! far gateway where nothing moves that is the model's slow pace, as it
//! is for the polls (ticket 72). The polls leave it out and stay quick.
//!
//! A lane only reads. It opens its socket as every socket is opened, asks
//! the model for a reading of its one session (`Gateway::poll_apart`),
//! runs it as a poll is run, and hands it back; the model drops one that
//! was begun before something it could not know of. It tells the model
//! nothing else: a lane's socket failing says nothing of the gateway.
//!
//! A lane ends when its follower does, when the gateway is no longer
//! ready, when the polls have not wanted the session read for a while
//! (it went idle, or left the list), or when its reads are quick again,
//! and the polls take the session back. A lane that ended is not a lost
//! session: the next poll reads it, and finds it slow again or not.
use super::{
    opening::opened,
    polls::{rounds, stands},
    socket::Socket,
    Pace, Shared,
};
use agent_hud_agents::hermes::gateway::{Connection, Gateway, Input, SessionRef, Status};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// The most sessions of one gateway read apart at a time. One more that
/// is slow stays with the polls.
const MOST: usize = 4;
/// How many reads in a row have to be quick before the polls take the
/// session back.
const QUICK: u32 = 3;
/// How many polls may go by without wanting the session read before its
/// lane ends, in the polls' own pace (`every`), on top of the time one
/// call may take.
const UNWANTED: u32 = 6;
/// How many times in a row a lane may fail to open its socket.
const TRIES: u32 = 3;

/// The lanes of one follower: each session read apart, with when a poll
/// last wanted it read.
#[derive(Default)]
pub(super) struct Lanes {
    lanes: Mutex<HashMap<SessionRef, Instant>>,
}

impl Lanes {
    fn lanes(&self) -> MutexGuard<'_, HashMap<SessionRef, Instant>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `session` is read apart. A poll asks when it would read
    /// the session itself, which is also what keeps the lane going.
    pub(super) fn reads(&self, session: &SessionRef) -> bool {
        let mut lanes = self.lanes();
        match lanes.get_mut(session) {
            Some(wanted) => {
                *wanted = Instant::now();
                true
            }
            None => false,
        }
    }

    /// How many sessions are read apart, for the tests.
    #[cfg(test)]
    pub(super) fn count(&self) -> usize {
        self.lanes().len()
    }

    /// Reads `session` apart from now on, where it is not already and
    /// there is room for one more.
    pub(super) fn begin(&self, shared: &Arc<Shared>, session: SessionRef) {
        let mut lanes = self.lanes();
        if lanes.len() >= MOST || lanes.contains_key(&session) || !shared.active() {
            return;
        }
        lanes.insert(session.clone(), Instant::now());
        drop(lanes);
        let line = format!("apart session={}", session.hud_id());
        shared.log.line(&line);
        let (owned, read) = (shared.clone(), session.clone());
        let started = thread::Builder::new()
            .name("Hermes slow session".into())
            .spawn(move || work(&owned, &read));
        if started.is_err() {
            self.lanes().remove(&session);
        }
    }

    fn wanted(&self, session: &SessionRef) -> Option<Instant> {
        self.lanes().get(session).copied()
    }
}

/// How long between the starts of two polls of the gateway just now, as
/// `link.rs` paces them: the close watch while the model keeps it, and
/// the model's own wait otherwise. A lane reads no more often, and is
/// kept for as many polls, whichever pace they come at. Without this a
/// session listed as working for days and slow to read was read back to
/// back while its gateway was polled every ten seconds, and its lane
/// could end between two such polls.
fn every(gateway: &Gateway, pace: &Pace) -> Duration {
    let wait = Duration::from_millis(gateway.poll_ms().max(0) as u64);
    match gateway.lively() {
        true => wait.min(pace.watch),
        false => wait,
    }
}

/// How far apart the polls come that would want `session` read, for
/// how long its lane is kept: as often as the gateway is polled while
/// a poll would read the session. Once none would (it left the live
/// list, or is listed idle and shows no card), no poll is waited for,
/// and the lane ends as it did before the polls could come ten seconds
/// apart: after six of the close watch (ticket 73). It sends nothing
/// meanwhile, and held a thread, a socket and a lane's place for 65 s.
fn kept(gateway: &Gateway, session: &SessionRef, pace: &Pace) -> Duration {
    match gateway.status(session) {
        Some(Status::Working | Status::Waiting) => every(gateway, pace),
        _ => pace.watch,
    }
}

/// Whether a lane no poll has wanted its session read by for `since`
/// is over, where the polls come `every` so often. A poll can itself
/// be slow, by as much as a call may take.
fn unwanted(since: Duration, every: Duration, pace: &Pace) -> bool {
    since > every * UNWANTED + pace.answer
}

/// Reads one session, apart, until its lane ends.
fn work(shared: &Arc<Shared>, session: &SessionRef) {
    let pace = shared.pace;
    let mut socket: Option<Socket> = None;
    let (mut quick, mut failed) = (0, 0);
    let why = loop {
        if !shared.active() {
            break "ended";
        }
        let wanted = shared.apart.wanted(session);
        let every = || every(&shared.model().gateway, &pace);
        let kept = || kept(&shared.model().gateway, session, &pace);
        if wanted.is_none_or(|at| unwanted(at.elapsed(), kept(), &pace)) {
            break "unwanted";
        }
        let instance = match shared.model().gateway.connection() {
            Connection::Ready(instance) => instance.clone(),
            _ => break "unready",
        };
        let began = Instant::now();
        if socket.is_none() {
            socket = opened(shared, &instance).ok();
        }
        let Some(open) = socket.as_mut() else {
            failed += 1;
            if failed >= TRIES {
                break "unopened";
            }
            shared.pause(pace.retry_limit);
            continue;
        };
        failed = 0;
        let reading = shared.model().gateway.poll_apart(session);
        let Some(mut reading) = reading else {
            // Nothing to read of it just now.
            shared.pause(pace.gap);
            continue;
        };
        match rounds(shared, open, &instance, &mut reading, false) {
            Ok(Some(asked)) => {
                #[cfg(test)]
                shared.late.pass_apart();
                let mut model = shared.model();
                if model.stopped {
                    break "ended";
                }
                // Read over a link that is gone by now (`polls.rs`):
                // dropped, and the lane's socket is opened again.
                if !stands(shared, open) {
                    drop(model);
                    if let Some(mut lost) = socket.take() {
                        lost.close();
                    }
                    shared.pause(pace.retry);
                    continue;
                }
                shared.take(&mut model, Input::Polled(reading));
                drop(model);
                let took = began.elapsed();
                if pace.trace {
                    let (ms, calls) = (took.as_millis(), asked.calls);
                    let line = format!("trace apart ms={ms} calls={calls}");
                    shared.log.line(&line);
                }
                quick = match took < pace.apart {
                    true => quick + 1,
                    false => 0,
                };
                if quick >= QUICK {
                    break "quick";
                }
                // No more often than the polls come, looked at again
                // now and then: a gateway where something moves is
                // polled quickly from that moment, and so is this read.
                while shared.active() {
                    let left = every().saturating_sub(began.elapsed());
                    if left.is_zero() {
                        break;
                    }
                    shared.pause(left.min(pace.gap));
                }
            }
            Ok(None) if !shared.active() => break "ended",
            // The reading counts no more and was given up. A call of it
            // may still be out on this socket, so the next reading gets
            // a socket of its own.
            Ok(None) => {
                if let Some(mut stale) = socket.take() {
                    stale.close();
                }
                shared.pause(pace.gap);
            }
            // The lane's own socket: it is opened again, and nothing is
            // said of the gateway.
            Err(_) => {
                if let Some(mut lost) = socket.take() {
                    lost.close();
                }
                shared.pause(pace.retry);
            }
        }
    };
    if let Some(mut open) = socket {
        open.close();
    }
    shared.apart.lanes().remove(session);
    if shared.active() {
        let id = session.hud_id();
        shared.log.line(&format!("together session={id} why={why}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_agents::hermes::gateway::{
        machine_key, GatewayId, Machine, Profile, Step, IDLE_POLL_MS, QUIET_MS, SESSION_AUTHORITY,
    };
    use serde_json::json;

    /// A gateway of `machine` that lists one session as working, polled
    /// at `at` and at each time after it with the session as it was.
    fn polled(machine: Machine, at: &[i64]) -> Gateway {
        listing(machine, at, "working")
    }

    /// The same, with the session listed as `status` at the last poll,
    /// or with no session listed at it where `status` is empty.
    fn listing(machine: Machine, at: &[i64], status: &str) -> Gateway {
        let mut gateway = Gateway::new(GatewayId::new(machine, Profile::Default));
        let answer = json!({"runtime_protocol": 1, "state": "ready", "instance_id": "one",
            "authority_epoch": 1, "api_origin": "http://127.0.0.1:54321",
            "capabilities": [SESSION_AUTHORITY],
            "served_profiles": [{"profile_id": "/synthetic", "home": "/synthetic"}]});
        let (answer, home) = (&answer, "/synthetic");
        gateway.apply(Input::Identified { answer, home }, 0);
        for (n, now) in at.iter().enumerate() {
            let mut poll = gateway.poll().expect("a gateway that is ready");
            let rows = match (n + 1 == at.len(), status) {
                (true, "") => json!({"sessions": []}),
                (true, status) => json!({"sessions": [{"id": "s-stuck", "status": status}]}),
                _ => json!({"sessions": [{"id": "s-stuck", "status": "working"}]}),
            };
            let place = json!({"events": [], "replay_epoch": "epoch-1", "last_sequence": 0});
            for _ in 0..8 {
                for step in poll.steps() {
                    let list = matches!(step, Step::List(_));
                    poll.answer(&step, if list { &rows } else { &place });
                }
            }
            assert!(poll.steps().is_empty());
            gateway.apply(Input::Polled(poll), *now);
        }
        gateway
    }

    fn far() -> Machine {
        let install = "0123456789abcdef0123456789abcdef";
        Machine::Far(machine_key(Some(install), "/Users/USER/.hermes").expect("a made-up identity"))
    }

    /// A lane reads as often as its gateway is polled: at the close
    /// watch while something moved lately, and at the model's slow pace
    /// on a far gateway where nothing has for the quiet time. A gateway
    /// of this machine keeps its close watch while it lists anything.
    #[test]
    fn a_lane_reads_at_the_pace_its_gateway_is_polled_at() {
        let pace = Pace::default();
        let slow = Duration::from_millis(IDLE_POLL_MS as u64);
        assert_eq!(every(&polled(far(), &[0]), &pace), pace.watch);
        assert_eq!(every(&polled(far(), &[0, QUIET_MS - 1]), &pace), pace.watch);
        assert_eq!(every(&polled(far(), &[0, QUIET_MS]), &pace), slow);
        let here = polled(Machine::This, &[0, QUIET_MS]);
        assert_eq!(every(&here, &pace), pace.watch);
    }

    /// A lane lives from one slow poll to the next only while a poll
    /// would read its session. Once the session has left the list, or is
    /// listed idle with no card, the lane ends after six polls of the
    /// close watch, as it did before a gateway could be polled every
    /// ten seconds: on this machine too, whose polls slow down when the
    /// list empties.
    #[test]
    fn a_lane_no_poll_would_read_the_session_of_ends_at_the_close_watch() {
        let pace = Pace::default();
        let slow = Duration::from_millis(IDLE_POLL_MS as u64);
        let at = |gateway: &Gateway| gateway.session(&Profile::Default, "s-stuck").unwrap();
        let kept = |gateway: &Gateway| kept(gateway, &at(gateway), &pace);
        // Listed as working on a far gateway where nothing moves: the
        // polls that want it read come ten seconds apart.
        let quiet = polled(far(), &[0, QUIET_MS]);
        assert_eq!((every(&quiet, &pace), kept(&quiet)), (slow, slow));
        assert!(!unwanted(Duration::from_secs(65), kept(&quiet), &pace));
        // Gone from the list: the gateway is polled as slowly, on this
        // machine too, and no poll is coming that would want it read.
        for machine in [far(), Machine::This] {
            let left = listing(machine, &[0, QUIET_MS, QUIET_MS + 10_000], "");
            assert_eq!(every(&left, &pace), slow);
            assert_eq!(kept(&left), pace.watch);
            assert!(unwanted(Duration::from_millis(9_801), kept(&left), &pace));
        }
        // Listed idle, with no card: no poll reads it either.
        let idle = listing(far(), &[0, QUIET_MS, QUIET_MS + 10_000], "idle");
        assert_eq!(kept(&idle), pace.watch);
        // Waiting, where it worked: read, and kept, at the quick pace.
        let waits = listing(far(), &[0, QUIET_MS, QUIET_MS + 10_000], "waiting");
        assert_eq!(kept(&waits), pace.watch);
        assert!(!unwanted(Duration::from_millis(9_800), kept(&waits), &pace));
    }

    /// A lane is kept for six polls and the time one call may take, at
    /// the pace the polls come at: 9.8 s at the close watch, and 65 s
    /// where they come ten seconds apart, so that it lives from one such
    /// poll to the next.
    #[test]
    fn a_lane_is_kept_for_six_polls_whichever_pace_they_come_at() {
        let pace = Pace::default();
        let (ms, slow) = (Duration::from_millis, Duration::from_secs(10));
        assert!(!unwanted(ms(9_800), pace.watch, &pace));
        assert!(unwanted(ms(9_801), pace.watch, &pace));
        assert!(!unwanted(ms(10_500), slow, &pace), "the next slow poll");
        assert!(!unwanted(ms(65_000), slow, &pace));
        assert!(unwanted(ms(65_001), slow, &pace));
    }
}
