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
use super::{socket::result, socket::Socket, visit::Visit, Model, Shared};
use agent_hud_agents::hermes::gateway::{
    frame, hints, Connection, Failure, Frame, Input, Instance, Poll, Refusal, Scope, SessionRef,
    Step,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc},
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
    #[cfg(test)]
    shared.late.pass();
    let mut model = shared.model();
    if model.stopped {
        return Ok(None);
    }
    // Read over a link that is gone by now: the poll is dropped, and its
    // socket with it, which was that link's.
    if !stands(shared, socket) {
        return Err(Failure::Unreachable);
    }
    shared.take(&mut model, Input::Polled(poll));
    let wait = Duration::from_millis(model.gateway.poll_ms().max(0) as u64);
    let live = !model.gateway.statuses().is_empty();
    // What the list says of a far machine's live sessions, for their
    // rows: copied here, and queued for the mirrors' writer once the
    // model is unlocked. The writer drops a list it has taken already.
    let listing = (shared.mirror.as_ref()).and_then(|_| model.gateway.listing());
    let working = model.gateway.working();
    // What a session has used is asked on a lane (`counting.rs`).
    let counts = model.gateway.count().is_some();
    drop(model);
    if let (Some(mirror), Some(listing)) = (&shared.mirror, listing) {
        // One of them is in a turn: the machine's history is read at
        // its quick pace for as long as a poll says so.
        if working {
            mirror.worked(listing.read_ms);
        }
        mirror.listed(listing, shared.under.load(Ordering::Acquire));
    }
    if counts {
        shared.counts.want(shared);
    }
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

/// Whether what was read on `socket` may still be taken: the link it
/// was read over is the one that stands now. A reading carries the link
/// of its socket, and one whose link was lost since, or gave way to
/// another, is older than that loss. What is known since (a card marked
/// as out of reach when an answer could not be sent) is never undone by
/// it: the reading is dropped whole. Asked with the model locked, in the
/// step that would take the reading, so no mark can come between.
pub(super) fn stands(shared: &Shared, socket: &Socket) -> bool {
    shared.route.epoch() == Some(socket.on())
}

/// What the rounds of one poll asked.
pub(super) struct Asked {
    pub(super) calls: usize,
    pub(super) snapshots: usize,
    /// The sessions whose read took long enough to be read apart.
    slow: Vec<SessionRef>,
}

/// How long one look at the socket lasts while a reading of one session
/// waits for a reply: a reading that counts no more is given up within
/// this.
const GLANCE: Duration = Duration::from_millis(50);

/// Asks every step `poll` wants, round after round, and gives it each
/// answer. An error when the socket was lost. `beside` is whether this
/// is a poll, which leaves out the sessions that are read apart, or a
/// reading of one of those.
///
/// None where nothing is to be handed back: the follower ended
/// meanwhile, or this is a reading of one session that counts no more.
/// Such a reading sends nothing further once the model would not take
/// it: its session left the live list, a poll lists it as something
/// else that is not waiting, an answer was taken. That is asked under the model's
/// lock before every call on `socket`, now and then while a reply is
/// waited for, and for a snapshot in the step that marks its visit as
/// writing (`visit.rs`), with the `session.resume` the next thing
/// written: a snapshot subscribes, and a session off the live list is
/// never subscribed to.
/// A reading given up may have a call still out on `socket`, which is
/// then of no further use.
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
            // held comes back to no next round, and no next call. Nor
            // does a reading that counts no more.
            if !shared.active() || !(beside || counts(shared, poll)) {
                return Ok(None);
            }
            // Read apart: nothing is known of it by this poll, which
            // leaves its cards as they are.
            // A look for the turns is no reading of a session's cards:
            // it goes to no lane, and keeps none going.
            let session = match &step {
                Step::Since { session, .. } | Step::Snapshot(session) => Some(session),
                // No step of a poll: counts have a lane (`counting.rs`).
                Step::List(_) | Step::Watch { .. } | Step::Stats(_) => None,
            };
            if beside && session.is_some_and(|session| shared.apart.reads(session)) {
                poll.failed(&step);
            } else if shared.rests.holds(&step) {
                // Its look was slow a moment ago (`turns.rs`).
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
            let value = match beside {
                true => socket.read(until)?,
                // Looked at now and then, so that a reading that counts
                // no more does not wait out a slow reply.
                false => match socket.read(until.min(Instant::now() + GLANCE))? {
                    None if Instant::now() < until && shared.active() => {
                        if !counts(shared, poll) {
                            return Ok(None);
                        }
                        continue;
                    }
                    value => value,
                },
            };
            let Some(value) = value else {
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
                        if took >= shared.pace.apart {
                            shared.rests.rest(&step, shared.pace.answer);
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
            shared.rests.rest(&step, shared.pace.answer);
            poll.failed(&step);
        }
        for step in alone {
            if !shared.active() {
                return Ok(None);
            }
            snapshots += 1;
            // A reading of one session is asked whether it still counts
            // in the step before the resume is written: its socket takes
            // a moment to open, and the model may know more by then.
            let reading: &Poll = poll;
            let still = move |model: &Model| beside || model.gateway.counts(reading);
            match snapshot(shared, instance, &step, &still) {
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

/// Whether the model would still take `poll`, asked under its lock.
fn counts(shared: &Shared, poll: &Poll) -> bool {
    shared.model().gateway.counts(poll)
}

/// Everything that waits in one session, from a visit that ends at once.
/// `still` is asked under the model's lock before anything subscribes.
fn snapshot(
    shared: &Shared,
    instance: &Instance,
    step: &Step,
    still: &dyn Fn(&Model) -> bool,
) -> Option<Value> {
    let Step::Snapshot(session) = step else {
        return None;
    };
    let wait = shared.pace.answer * 2;
    let (visit, told) = Visit::begin_while(shared, instance, session, wait, &|| true, still)?;
    visit.end();
    Some(told)
}
