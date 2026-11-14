//! The connection to one gateway over its life: ask the control socket who
//! is there, open a socket (`opening.rs`), run polls until it is lost,
//! then begin again. Where the gateway is, the route knows (`route.rs`):
//! nothing here names a path or an address. Nothing is kept from one dial
//! to the next but the model.
//!
//! A follower is one gateway's. Where its home answers as another
//! instance than the one it follows, that is not its gateway any more,
//! and what it is instead takes a look at every home of the machine
//! (`discovery.rs`): a restart ends this follower and makes the next. So
//! such an answer is not taken; it is said, and asked again.
use super::{
    control::{self, Silent},
    cost::Told,
    opening::{opened, Shut},
    polls,
    route::Home,
    socket::Socket,
    Shared,
};
use agent_hud_agents::hermes::gateway::{
    hints, identify, identify_line, Connection, Failure, Identified, Input, Instance,
};
use serde_json::Value;
use std::{
    sync::{atomic::Ordering, Arc, PoisonError},
    time::{Duration, Instant},
};

/// How long one look at the socket lasts while waiting for the next poll:
/// a poll that is wanted starts within this.
const GLANCE: Duration = Duration::from_millis(50);

/// What the next dial waits for.
enum Next {
    /// The gateway is starting or draining: ask again soon.
    Soon,
    /// It cannot be reached or refused: wait longer each time.
    Later,
    /// It has no live connection: ask rarely.
    Rarely,
}

pub(super) fn work(shared: Arc<Shared>) {
    let mut wait = shared.pace.retry;
    // Said once for each run of dials, so a gateway that stays away does
    // not turn the state back and forth. A follower made from a gateway's
    // answer has that answer in hand and begins with it.
    let mut dialing = born(&shared).is_some();
    while shared.active() {
        if !dialing {
            shared.apply(Input::Dialing);
            dialing = true;
        }
        let next = match dial(&shared) {
            Ok(mut socket) => {
                (wait, dialing) = (shared.pace.retry, false);
                if let Some(failure) = follow(&shared, &mut socket) {
                    shared.apply(Input::Lost(failure));
                    // What became of the gateway is looked for, by a
                    // follower that still follows it.
                    if shared.active() {
                        shared.look.want();
                    }
                }
                socket.close();
                Next::Soon
            }
            Err(next) => next,
        };
        let pause = match next {
            Next::Soon => shared.pace.retry,
            Next::Rarely => shared.pace.unsupported,
            Next::Later => {
                wait = (wait * 2).min(shared.pace.retry_limit);
                wait
            }
        };
        shared.pause(pause);
    }
}

/// The home whose gateway is followed, as it is now: the one this
/// follower asks, among the homes the route tells of.
pub(super) fn home(shared: &Shared) -> Result<Home, Silent> {
    let (homes, asked) = (shared.route.homes()?, shared.asked());
    let mut homes = homes.into_iter();
    homes
        .find(|home| *home.profile() == asked)
        .ok_or(Silent::Absent)
}

/// The answer this follower was made from, until its first identify
/// takes it.
fn born(shared: &Shared) -> std::sync::MutexGuard<'_, Option<Value>> {
    shared.born.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asks the control socket of that home who is there, once more where it
/// only was slow: identify can stall on a busy gateway and answer at once
/// the next time. The answer comes with the home that was asked. A
/// follower made from an answer takes that one first.
///
/// This is where a follower learns that its gateway is gone: its socket
/// is lost, or it would not be asking, and nobody listens at the control
/// socket of its home. A home that cannot be told of, and a control
/// socket that is there and does not answer, say no such thing.
fn asked(shared: &Shared) -> Result<(Value, Home), Silent> {
    // Said once the asking is over, and never in between: a follower
    // whose gateway is gone stays so for as long as nobody listens.
    let gone = |gone: bool| shared.gone.store(gone, Ordering::Release);
    let home = home(shared).inspect_err(|_| gone(false))?;
    if let Some(answer) = born(shared).take() {
        gone(false);
        return Ok((answer, home));
    }
    let line = identify_line();
    let ask = || control::ask(&*shared.route, &home, &line, shared.pace.answer);
    let answer = match ask() {
        Err(Silent::TimedOut) => ask(),
        answer => answer,
    };
    gone(matches!(answer, Err(Silent::Absent)));
    Ok((answer?, home))
}

/// Whether `answer` is another instance than the one this follower
/// follows. It is kept, and a look at the whole machine is asked for.
fn elsewhere(shared: &Shared, answer: &Value, home: &str) -> bool {
    let Identified::Ready(now) = identify(answer, home) else {
        return false;
    };
    let follows = |bound: &str| bound != now.id;
    if !shared.model().gateway.instance().is_some_and(follows) {
        return false;
    }
    *shared.found.lock().unwrap_or_else(PoisonError::into_inner) = Some(now.id);
    shared.look.want();
    true
}

pub(super) fn silent(silent: Silent) -> Failure {
    match silent {
        Silent::Absent | Silent::TimedOut => Failure::Unreachable,
        Silent::Unsafe(what) => Failure::Refused(format!("the control {what} is not private")),
        Silent::Garbled => Failure::Invalid("invalid_control_response"),
    }
}

/// Identifies the gateway and tells the model, which says whether it is
/// one to connect to.
fn identified(shared: &Shared) -> Result<Instance, Next> {
    let (answer, home) = match asked(shared) {
        Ok(answered) => answered,
        Err(why) => {
            shared.apply(Input::Lost(silent(why)));
            return Err(Next::Later);
        }
    };
    // The home is compared as the gateway wrote it, links resolved, and
    // resolved after the answer came: a home that leads elsewhere by then
    // is not the one this gateway serves.
    let home = shared.route.named(&home);
    if elsewhere(shared, &answer, &home) {
        return Err(Next::Soon);
    }
    let held = shared.model().gateway.holds().clone();
    shared.apply(Input::Identified {
        answer: &answer,
        home: &home,
    });
    let model = shared.model();
    // The gateway serves other profiles than it did: which gateway has
    // which is looked at again.
    if !held.is_empty() && *model.gateway.holds() != held {
        shared.look.want();
    }
    match model.gateway.connection() {
        Connection::Ready(instance) => Ok(instance.clone()),
        Connection::Connecting => Err(Next::Soon),
        Connection::Unsupported(_) => Err(Next::Rarely),
        Connection::Disconnected | Connection::Failed(_) => Err(Next::Later),
    }
}

fn dial(shared: &Shared) -> Result<Socket, Next> {
    let instance = identified(shared)?;
    opened(shared, &instance).map_err(|shut| {
        match shut {
            Shut::Denied(denied) => shared.apply(Input::Denied(denied)),
            Shut::Failed(failure) => shared.apply(Input::Lost(failure)),
            Shut::Stopped => {}
        }
        match shared.model().gateway.connection() {
            Connection::Unsupported(_) => Next::Rarely,
            _ => Next::Later,
        }
    })
}

/// Runs polls on the socket until it is lost (the failure) or the observer
/// stops (None). The first poll starts at once: at startup and after every
/// reconnect.
fn follow(shared: &Shared, socket: &mut Socket) -> Option<Failure> {
    let mut due = Instant::now();
    let mut last: Option<Instant> = None;
    let mut told = Told::default();
    // Until when polls keep the close watch after someone asked for one.
    let mut warm: Option<Instant> = None;
    // The first poll waits for the first look at the machine, which says
    // which profiles two gateways claim.
    while !shared.look.looked() {
        if !shared.active() {
            return None;
        }
        shared.pause(GLANCE);
    }
    loop {
        // Wait for the timer or for someone to want a poll, reading the
        // socket meanwhile: Hermes says there when its sessions changed.
        loop {
            if !shared.active() {
                return None;
            }
            let now = Instant::now();
            let spaced = last.is_none_or(|last| now >= last + shared.pace.gap);
            if spaced && (now >= due || shared.is_wanted()) {
                if shared.take_wanted() {
                    warm = Some(now + shared.pace.warm);
                }
                break;
            }
            match socket.read(now + GLANCE) {
                Ok(Some(frame)) if hints(&frame) => {
                    if shared.pace.trace {
                        let kind = frame["params"]["type"].as_str().unwrap_or("request");
                        shared.log.line(&format!("trace hint {kind}"));
                    }
                    shared.want_poll()
                }
                Ok(_) => {}
                Err(failure) => return Some(failure),
            }
        }
        let began = Instant::now();
        last = Some(began);
        match polls::run(shared, socket) {
            // The wait runs from when the poll began, so a busy gateway is
            // asked once a second and not once a second plus a poll.
            Ok(Some(ran)) => {
                if shared.pace.trace {
                    shared.log.line(&format!(
                        "trace poll ms={} calls={} snapshots={} next_ms={}",
                        ran.took.as_millis(),
                        ran.calls,
                        ran.snapshots,
                        ran.wait.as_millis()
                    ));
                }
                let close = ran.live || warm.is_some_and(|until| began < until);
                let wait = if close {
                    ran.wait.min(shared.pace.watch)
                } else {
                    ran.wait
                };
                due = began + wait;
                told.add(shared, &ran);
            }
            // The model is no longer ready to poll: begin again.
            Ok(None) => return Some(Failure::Unreachable),
            Err(failure) => return Some(failure),
        }
    }
}
