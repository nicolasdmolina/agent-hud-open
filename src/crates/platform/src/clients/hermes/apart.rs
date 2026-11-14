//! The sessions that are read apart from the polls. A session whose
//! events take long to read (a long stream over a slow link, a gateway
//! busy with that session) would hold up every poll that reads it, and
//! so every other session's cards. Once a poll has found a read that
//! slow (`polls.rs`), the session gets a lane: a thread and a socket of
//! its own, on which it is read by itself, again and again, for as long
//! as the polls would read it. The polls leave it out and stay quick.
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
use super::{opening::opened, polls::rounds, socket::Socket, Shared};
use agent_hud_agents::hermes::gateway::{Connection, Input, SessionRef};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::Instant,
};

/// The most sessions of one gateway read apart at a time. One more that
/// is slow stays with the polls.
const MOST: usize = 4;
/// How many reads in a row have to be quick before the polls take the
/// session back.
const QUICK: u32 = 3;
/// How many polls may go by without wanting the session read before its
/// lane ends, in the polls' own pace (`Pace::watch`), on top of the time
/// one call may take.
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
        // A poll can itself be slow, by as much as a call may take.
        if wanted.is_none_or(|at| at.elapsed() > pace.watch * UNWANTED + pace.answer) {
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
                let mut model = shared.model();
                if model.stopped {
                    break "ended";
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
                // No more often than the polls read a busy session.
                shared.pause(pace.watch.saturating_sub(took));
            }
            Ok(None) => break "ended",
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
