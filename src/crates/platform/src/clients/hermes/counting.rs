//! What a far machine's sessions have used so far, asked apart from the
//! polls (`agents::hermes::gateway`, `sample.rs`). A count is one call
//! for each session that works, and Hermes can be slow to say one. A
//! gateway answers the calls of one socket in turn, so a count asked on
//! the polls' socket holds up every list and every read behind it, and
//! with them every card. So the count calls have a lane: a thread and
//! a socket of their own, on which nothing else is asked.
//!
//! A round of them follows a poll, never the other way about. Each
//! poll that came back with somebody to ask wants one round, and one
//! round runs at a time: a round that is slow makes the rounds fewer,
//! and no poll later. After a round the lane rests for as long as the
//! round took, so a gateway that is slow to count is asked for at most
//! half of the time.
//!
//! Each call of a round may take as long as any call may
//! (`Pace::answer`). One that takes longer is given up, and the
//! lane's socket with it, since the call is still out on it; the next
//! round opens another. What a round learned goes to the model, which
//! notes who is still owed a count, and its samples to the mirrors'
//! writer (`mirror.rs`), under the route number the lane's socket was
//! opened under. A round read over a link that is gone by now is
//! dropped whole (`polls::stands`).
//!
//! The lane tells the model nothing of the gateway: its socket failing
//! says nothing of the connection the polls run on, and a socket that
//! did not open is tried again no sooner than a gateway that stays
//! away is dialled again. It ends when its follower does, and when no
//! poll has wanted a round for a while, which closes its socket; the
//! next poll that wants one starts it anew. Its socket is closed too
//! once the route number is no longer the one it was opened under,
//! whether or not a round is wanted (ticket 64): the next round opens
//! one over the route that is the machine's now.
use super::{opening::opened, polls::stands, socket::result, socket::Socket, Shared};
use agent_hud_agents::hermes::gateway::{
    frame, Connection, Counting, Failure, Frame, Input, Scope, Step,
};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// How long the lane keeps its socket with no round wanted, in the
/// times one call may take (`Pace::answer`): thirty seconds in the app.
const LINGER: u32 = 6;
/// How long one wait for a poll lasts before the lane looks whether its
/// follower ended.
const GLANCE: Duration = Duration::from_millis(100);

#[derive(Default)]
struct State {
    /// The lane's thread runs.
    running: bool,
    /// A poll came back with somebody to ask since the last round began.
    wanted: bool,
}

/// The lane of one follower.
#[derive(Default)]
pub(super) struct Lane {
    state: Mutex<State>,
    wake: Condvar,
    /// How many rounds ran to their end, for the tests.
    #[cfg(test)]
    pub(super) rounds: std::sync::atomic::AtomicUsize,
}

impl Lane {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A poll came back and the model has somebody to ask: one round is
    /// wanted, however many polls say so before it begins. It returns
    /// at once; the lane's thread is started where it does not run.
    pub(super) fn want(&self, shared: &Arc<Shared>) {
        let mut state = self.state();
        state.wanted = true;
        if !state.running && shared.active() {
            let owned = shared.clone();
            let started = thread::Builder::new()
                .name("Hermes counts".into())
                .spawn(move || work(&owned));
            state.running = started.is_ok();
        }
        drop(state);
        self.wake.notify_all();
    }

    /// Waits for a round to be wanted, or for the route number to be
    /// other than `opened`, the one the lane's socket was opened under.
    /// `Over` once no round was wanted for `linger`, or the follower
    /// ended: the lane then runs no more, said in the step that found
    /// so, so that a poll that wants a round after it starts the lane
    /// anew.
    fn wanted(&self, shared: &Shared, linger: Duration, opened: Option<u64>) -> Wake {
        let until = Instant::now() + linger;
        loop {
            // Asked with the lane unlocked: the route holds its number
            // while a mirror commits, and a poll that wants a round
            // must not wait on the disk.
            let moved = opened.is_some_and(|number| shared.route.number() != number);
            let mut state = self.state();
            let left = until.saturating_duration_since(Instant::now());
            if !shared.active() || (!state.wanted && left.is_zero()) {
                state.running = false;
                return Wake::Over;
            }
            if moved {
                return Wake::Moved;
            }
            if state.wanted {
                state.wanted = false;
                return Wake::Round;
            }
            drop(self.wake.wait_timeout(state, left.min(GLANCE)));
        }
    }
}

/// What the lane does next.
enum Wake {
    /// A round is wanted.
    Round,
    /// The route number is not the one its socket was opened under.
    Moved,
    /// The lane ends.
    Over,
}

/// Asks every call of `counting` on `socket` and gives it each answer.
/// False where a call was never answered: it is still out on the
/// socket, which is of no further use. An error where the socket was
/// lost.
fn round(shared: &Shared, socket: &mut Socket, counting: &mut Counting) -> Result<bool, Failure> {
    let mut asked: HashMap<u64, Step> = HashMap::new();
    for step in counting.steps() {
        asked.insert(socket.send(&step.command(Scope::Host))?, step);
    }
    // Each reply is waited for from the one before it: a gateway
    // answers the calls of one socket in turn.
    let mut until = Instant::now() + shared.pace.answer;
    while !asked.is_empty() && shared.active() {
        let Some(value) = socket.read(until)? else {
            break;
        };
        let Frame::Reply { id } = frame(&value) else {
            continue;
        };
        if let Some(step) = asked.remove(&id) {
            until = Instant::now() + shared.pace.answer;
            match result(&value) {
                Ok(told) => counting.answer(&step, told),
                Err(_) => counting.failed(&step),
            }
        }
    }
    Ok(asked.is_empty())
}

/// Runs the rounds of count calls, one after each poll that wants one,
/// until the lane ends.
fn work(shared: &Arc<Shared>) {
    let pace = shared.pace;
    // The lane's socket, with the route number it was opened under.
    let mut socket: Option<(Socket, u64)> = None;
    // No round begins before this.
    let mut rest = Instant::now();
    loop {
        let under = socket.as_ref().map(|(_, number)| *number);
        match shared.counts.wanted(shared, pace.answer * LINGER, under) {
            Wake::Round => {}
            // Closed with no lock held: a poll that wants a round does
            // not wait on the network for it.
            Wake::Moved => {
                if let Some((mut old, _)) = socket.take() {
                    old.close();
                }
                continue;
            }
            Wake::Over => break,
        }
        shared.pause(rest.saturating_duration_since(Instant::now()));
        let begun = {
            let model = shared.model();
            match model.gateway.connection() {
                Connection::Ready(instance) if !model.stopped => {
                    (model.gateway.count()).map(|counting| (instance.clone(), counting))
                }
                _ => None,
            }
        };
        // Nobody to ask after all, or the gateway is not ready.
        let Some((instance, mut counting)) = begun else {
            continue;
        };
        let began = Instant::now();
        if socket.is_none() {
            // As the polls' socket is opened (`link.rs`): one opened
            // while the route changed is of no one route.
            let number = shared.route.number();
            match opened(shared, &instance) {
                Ok(open) if shared.route.number() == number => socket = Some((open, number)),
                Ok(mut open) => open.close(),
                Err(_) => {}
            }
        }
        // Not opened: tried again no sooner than a gateway that stays
        // away is dialled again, however many polls want a round.
        let Some((open, number)) = socket.as_mut() else {
            rest = Instant::now() + pace.retry_limit;
            continue;
        };
        let whole = round(shared, open, &mut counting).unwrap_or(false);
        // Whatever was not answered failed.
        for step in counting.steps() {
            counting.failed(&step);
        }
        let (samples, read_ms) = (counting.samples(), crate::hooks::now_ms());
        let mut model = shared.model();
        if shared.ending(&model) {
            break;
        }
        // Read over a link that is gone by now: dropped whole.
        let stood = stands(shared, open);
        if stood {
            shared.take(&mut model, Input::Counted(counting));
        }
        drop(model);
        #[cfg(test)]
        (shared.counts.rounds).fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let (Some(mirror), true) = (&shared.mirror, stood && !samples.is_empty()) {
            mirror.counted(samples, read_ms, *number);
        }
        if !(whole && stood) {
            if let Some((mut lost, _)) = socket.take() {
                lost.close();
            }
        }
        let took = began.elapsed();
        if pace.trace {
            let line = format!("trace counts ms={} whole={whole}", took.as_millis());
            shared.log.line(&line);
        }
        rest = Instant::now() + took;
    }
    if let Some((mut open, _)) = socket {
        open.close();
    }
}
