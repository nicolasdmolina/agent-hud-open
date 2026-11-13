//! Running one poll: ask every step the model wants, give it each answer,
//! and hand the whole poll back. The steps of one round do not depend on
//! each other, so they go out together and their replies are taken as they
//! come. A snapshot goes on a socket of its own (`visit.rs`). The model is
//! locked to begin the poll and to give it back, never in between.
use super::{socket::result, socket::Socket, visit::Visit, Shared};
use agent_hud_agents::hermes::gateway::{
    frame, hints, Connection, Failure, Frame, Input, Instance, Scope, Step,
};
use serde_json::Value;
use std::{
    collections::HashMap,
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
pub(super) fn run(shared: &Shared, socket: &mut Socket) -> Result<Option<Ran>, Failure> {
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
    let (mut calls, mut snapshots) = (0, 0);
    for _ in 0..ROUNDS {
        let steps = poll.steps();
        if steps.is_empty() {
            break;
        }
        let mut asked: HashMap<u64, Step> = HashMap::new();
        let mut alone = Vec::new();
        for step in steps {
            if matches!(step, Step::Snapshot(_)) {
                alone.push(step);
            } else {
                asked.insert(socket.send(&step.command(Scope::Host))?, step);
            }
        }
        calls += asked.len();
        let until = Instant::now() + shared.pace.answer;
        while !asked.is_empty() {
            let Some(value) = socket.read(until)? else {
                break;
            };
            match frame(&value) {
                Frame::Reply { id } => {
                    if let Some(step) = asked.remove(&id) {
                        match result(&value) {
                            Ok(told) => poll.answer(&step, told),
                            Err(_) => poll.failed(&step),
                        }
                    }
                }
                _ if hints(&value) => shared.want_poll(),
                _ => {}
            }
        }
        // Never answered: what they would have told stays unknown.
        for step in asked.into_values() {
            poll.failed(&step);
        }
        for step in alone {
            snapshots += 1;
            match snapshot(shared, &instance, &step) {
                Some(told) => poll.answer(&step, &told),
                None => poll.failed(&step),
            }
        }
    }
    for step in poll.steps() {
        poll.failed(&step);
    }
    let took = began.elapsed();
    let mut model = shared.model();
    if model.stopped {
        return Ok(None);
    }
    shared.take(&mut model, Input::Polled(poll));
    let wait = Duration::from_millis(model.gateway.poll_ms().max(0) as u64);
    Ok(Some(Ran {
        wait,
        live: !model.gateway.statuses().is_empty(),
        took,
        calls,
        snapshots,
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
