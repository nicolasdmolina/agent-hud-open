//! The connection to one gateway over its life: ask the control socket who
//! is there, get a ticket, open the socket, run polls until it is lost,
//! then begin again. A gateway that restarts comes back as another
//! instance, usually on another port, so nothing is kept from one dial to
//! the next but the model.
use super::{
    control::{self, Silent},
    polls,
    socket::Socket,
    Shared,
};
use agent_hud_agents::hermes::gateway::{
    granted, hints, identify_line, ticket_line, Connection, Denied, Failure, Input, Instance, Scope,
};
use serde_json::Value;
use std::{
    sync::Arc,
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
    // not turn the state back and forth.
    let mut dialing = false;
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

/// Asks the control socket one question, once more where it only was slow:
/// identify can stall on a busy gateway and answer at once the next time.
fn asked(shared: &Shared, line: &Value) -> Result<Value, Silent> {
    let home = std::fs::canonicalize(&shared.home).map_err(|_| Silent::Absent)?;
    match control::ask(&home, line, shared.pace.answer) {
        Err(Silent::TimedOut) => control::ask(&home, line, shared.pace.answer),
        answer => answer,
    }
}

fn silent(silent: Silent) -> Failure {
    match silent {
        Silent::Absent | Silent::TimedOut => Failure::Unreachable,
        Silent::Unsafe(what) => Failure::Refused(format!("the control {what} is not private")),
        Silent::Garbled => Failure::Invalid("invalid_control_response"),
    }
}

/// Identifies the gateway and tells the model, which says whether it is
/// one to connect to.
fn identified(shared: &Shared) -> Result<Instance, Next> {
    let answer = match asked(shared, &identify_line()) {
        Ok(answer) => answer,
        Err(why) => {
            shared.apply(Input::Lost(silent(why)));
            return Err(Next::Later);
        }
    };
    // The home is compared as the gateway wrote it, links resolved.
    let home = std::fs::canonicalize(&shared.home).unwrap_or_else(|_| shared.home.clone());
    shared.apply(Input::Identified {
        answer: &answer,
        home: &home.to_string_lossy(),
    });
    let model = shared.model();
    match model.gateway.connection() {
        Connection::Ready(instance) => Ok(instance.clone()),
        Connection::Connecting => Err(Next::Soon),
        Connection::Unsupported(_) => Err(Next::Rarely),
        Connection::Disconnected | Connection::Failed(_) => Err(Next::Later),
    }
}

/// Why no socket opened.
pub(super) enum Shut {
    Denied(Denied),
    Failed(Failure),
}

/// A new socket on `instance`, for every profile it serves: a ticket that
/// is asked for, used once and gone. Nothing is told to the model: a
/// socket for one answer or one snapshot failing says nothing of the
/// connection the polls run on.
pub(super) fn opened(shared: &Shared, instance: &Instance) -> Result<Socket, Shut> {
    let grant = asked(shared, &ticket_line(instance, Scope::Host))
        .map_err(|why| Shut::Failed(silent(why)))?;
    let ticket = granted(&grant, instance).map_err(Shut::Denied)?;
    drop(grant);
    Socket::open(instance, ticket).map_err(Shut::Failed)
}

fn dial(shared: &Shared) -> Result<Socket, Next> {
    let instance = identified(shared)?;
    opened(shared, &instance).map_err(|shut| {
        match shut {
            Shut::Denied(denied) => shared.apply(Input::Denied(denied)),
            Shut::Failed(failure) => shared.apply(Input::Lost(failure)),
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

/// What the polls cost, said in the log now and then: how many ran, how
/// long they took, how many calls and snapshots they made.
#[derive(Default)]
struct Told {
    polls: u32,
    calls: usize,
    snapshots: usize,
    total: Duration,
    longest: Duration,
}

impl Told {
    /// How many polls one log line sums up: a minute of a busy gateway.
    const EVERY: u32 = 60;
    /// A poll this slow is said at once.
    const SLOW: Duration = Duration::from_millis(500);

    fn add(&mut self, shared: &Shared, ran: &polls::Ran) {
        self.polls += 1;
        self.calls += ran.calls;
        self.snapshots += ran.snapshots;
        self.total += ran.took;
        self.longest = self.longest.max(ran.took);
        if ran.took >= Self::SLOW {
            shared.log.line(&format!(
                "slow poll ms={} calls={} snapshots={}",
                ran.took.as_millis(),
                ran.calls,
                ran.snapshots
            ));
        }
        if self.polls >= Self::EVERY {
            shared.log.line(&format!(
                "polls n={} mean_ms={} max_ms={} calls={} snapshots={}",
                self.polls,
                (self.total / self.polls).as_millis(),
                self.longest.as_millis(),
                self.calls,
                self.snapshots
            ));
            *self = Self::default();
        }
    }
}
