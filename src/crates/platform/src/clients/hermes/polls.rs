//! Running one poll: ask every step the model wants, give it each answer,
//! and hand the whole poll back. The steps of one round do not depend on
//! each other, so they go out together and their replies are taken as they
//! come. A snapshot goes on a socket of its own (`visit.rs`). The model is
//! locked to begin the poll and to give it back, never in between.
//!
//! A session whose events take long to read would hold every poll up, and
//! with it every other session's cards. So each read is timed, and a
//! session whose read took long is read apart from then on, on a socket
//! of its own (`apart.rs`): the polls leave it out, and go on at their
//! own pace.
use super::{socket::result, socket::Socket, visit::Visit, Shared};
use agent_hud_agents::hermes::gateway::{
    frame, hints, Connection, Failure, Frame, Input, Instance, Poll, Refusal, Scope, SessionRef,
    Step,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// More rounds than any poll takes: a list, the reads of one session one
/// after another (at most sixteen, when it has to search for the oldest
/// event Hermes still keeps), a snapshot.
const ROUNDS: usize = 24;

/// One poll that ran to its end.
pub(super) struct Ran {
    /// How long the model says to wait before the next one.
    pub(super) wait: Duration,
    /// Whether the gateway lists any live session.
    pub(super) live: bool,
    pub(super) took: Duration,
    pub(super) calls: usize,
    pub(super) snapshots: usize,
}

/// Runs one poll on `socket`. None when the gateway is not one to poll; an
/// error when the socket was lost, which drops the poll.
pub(super) fn run(shared: &Arc<Shared>, socket: &mut Socket) -> Result<Option<Ran>, Failure> {
    let (poll, instance) = {
        let mut model = shared.model();
        let instance = match model.gateway.connection() {
            Connection::Ready(instance) if !model.stopped => Some(instance.clone()),
            _ => None,
        };
        (model.gateway.poll(), instance)
    };
    let (Some(mut poll), Some(instance)) = (poll, instance) else {
        return Ok(None);
    };
    let began = Instant::now();
    let Some(asked) = rounds(shared, socket, &instance, &mut poll, true)? else {
        return Ok(None);
    };
    let took = began.elapsed();
    let mut model = shared.model();
    if model.stopped {
        return Ok(None);
    }
    shared.take(&mut model, Input::Polled(poll));
    let wait = Duration::from_millis(model.gateway.poll_ms().max(0) as u64);
    let live = !model.gateway.statuses().is_empty();
    drop(model);
    // A session whose read held this poll up is read apart from the next.
    for session in asked.slow {
        shared.apart.begin(shared, session);
    }
    Ok(Some(Ran {
        wait,
        live,
        took,
        calls: asked.calls,
        snapshots: asked.snapshots,
    }))
}

/// What the rounds of one poll asked.
pub(super) struct Asked {
    pub(super) calls: usize,
    pub(super) snapshots: usize,
    /// The sessions whose read took long enough to be read apart.
    slow: Vec<SessionRef>,
}

/// Asks every step `poll` wants, round after round, and gives it each
/// answer. None where the follower ended meanwhile; an error when the
/// socket was lost. `beside` is whether this is a poll, which leaves out
/// the sessions that are read apart, or a reading of one of those.
pub(super) fn rounds(
    shared: &Arc<Shared>,
    socket: &mut Socket,
    instance: &Instance,
    poll: &mut Poll,
    beside: bool,
) -> Result<Option<Asked>, Failure> {
    let (mut calls, mut snapshots, mut slow) = (0, 0, Vec::new());
    for _ in 0..ROUNDS {
        let steps = poll.steps();
        if steps.is_empty() {
            break;
        }
        let mut asked: HashMap<u64, Step> = HashMap::new();
        let mut alone = Vec::new();
        for step in steps {
            // A follower that ended asks nothing more: a reply that was
            // held comes back to no next round, and no next call.
            if !shared.active() {
                return Ok(None);
            }
            // Read apart: nothing is known of it by this poll, which
            // leaves its cards as they are.
            let session = match &step {
                Step::Since { session, .. } | Step::Snapshot(session) => Some(session),
                Step::List(_) => None,
            };
            if beside && session.is_some_and(|session| shared.apart.reads(session)) {
                poll.failed(&step);
            } else if matches!(step, Step::Snapshot(_)) {
                alone.push(step);
            } else {
                asked.insert(socket.send(&step.command(Scope::Host))?, step);
            }
        }
        calls += asked.len();
        let until = Instant::now() + shared.pace.answer;
        // Each reply is timed from the one before it: a gateway answers
        // the calls of one socket in turn, so the wait for a reply is the
        // time its own call took.
        let mut last = Instant::now();
        while !asked.is_empty() {
            let Some(value) = socket.read(until)? else {
                break;
            };
            match frame(&value) {
                Frame::Reply { id } => {
                    if let Some(step) = asked.remove(&id) {
                        let took = last.elapsed();
                        last = Instant::now();
                        if let Step::Since { session, .. } = &step {
                            if beside && took >= shared.pace.apart && !slow.contains(session) {
                                slow.push(session.clone());
                            }
                        }
                        match result(&value) {
                            Ok(told) => poll.answer(&step, told),
                            Err(refused) => {
                                // A profile that is not this gateway's:
                                // which gateway has it is looked for.
                                if refused == Refusal::ProfileMismatch {
                                    shared.look.want();
                                }
                                poll.failed(&step)
                            }
                        }
                    }
                }
                _ if hints(&value) => shared.want_poll(),
                _ => {}
            }
        }
        // Never answered: what they would have told stays unknown. One
        // that was a session's read held the poll up for as long as a
        // call may take.
        for step in asked.into_values() {
            if let Step::Since { session, .. } = &step {
                if beside && !slow.contains(session) {
                    slow.push(session.clone());
                }
            }
            poll.failed(&step);
        }
        for step in alone {
            if !shared.active() {
                return Ok(None);
            }
            snapshots += 1;
            match snapshot(shared, instance, &step) {
                Some(told) => poll.answer(&step, &told),
                None => poll.failed(&step),
            }
        }
    }
    for step in poll.steps() {
        poll.failed(&step);
    }
    Ok(Some(Asked {
        calls,
        snapshots,
        slow,
    }))
}

/// Everything that waits in one session, from a visit that ends at once.
fn snapshot(shared: &Shared, instance: &Instance, step: &Step) -> Option<Value> {
    let Step::Snapshot(session) = step else {
        return None;
    };
    let (visit, told) = Visit::begin(shared, instance, session, shared.pace.answer * 2)?;
    visit.end();
    Some(told)
}
