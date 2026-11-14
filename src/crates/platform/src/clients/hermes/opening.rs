//! Opening one socket on an instance: the route is told of the instance,
//! a ticket is asked for, a stream is dialled and upgraded with the
//! ticket. The polls' socket opens this way, and every visit's.
//!
//! Each of those steps waits on something, and the observer may stop
//! meanwhile (client hooks turned off). So every wait is followed by a
//! look at whether it has: once it has, nothing further is asked of the
//! gateway and nothing is dialled. The stream is the one thing a look
//! cannot cover, since it is used the moment it arrives: from then until
//! it is open it is on the model's list of sockets (`visit.rs`), put
//! there in the same locked step that looks at stop, so that stopping
//! cuts it. A stream that arrives once the observer stopped is closed
//! with nothing said on it.
//!
//! What this does not promise: a step that looked a moment before the
//! observer stopped still begins. One exchange with the control socket,
//! or one dial, can start in that instant; what it brings is dropped,
//! the stream unspoken on and the ticket unused. A ticket dropped that
//! way is not without effect at Hermes: it expires within the 30 s a
//! ticket lasts, and until then Hermes counts a client as attaching,
//! which can hold back a gateway's idle exit or a restart it has pending
//! for that long. And a socket whose upgrade ended just as the observer
//! stopped is off the list by then: its caller closes it, which writes a
//! WebSocket Close frame after the stop. That is tidying up, as a visit
//! that was not writing has always been let to do; no answer and no
//! `session.resume` goes out. The model is locked to list the stream and
//! to take it off, never across the route or the socket.
use super::{
    control::{self, Silent},
    link::{home, silent},
    socket::Socket,
    visit::Held,
    wire::Line,
    Model, Shared,
};
use agent_hud_agents::hermes::gateway::{granted, ticket_line, Denied, Failure, Instance, Scope};
use std::sync::Arc;

/// Why no socket opened.
pub(super) enum Shut {
    Denied(Denied),
    Failed(Failure),
    /// The observer stopped while the socket was being opened. Nothing
    /// more is asked of the gateway, and there is nothing to tell anyone.
    Stopped,
}

impl Model {
    /// Lists a stream that is about to be upgraded, so that stopping cuts
    /// it. None once the observer stopped: the stream is then not used.
    fn opening(&mut self, line: Arc<Line>) -> Option<u64> {
        if self.stopped {
            return None;
        }
        self.visited += 1;
        self.visits.insert(self.visited, Held::opening(line));
        Some(self.visited)
    }

    /// Takes the stream off the list, and says whether the observer still
    /// runs. Where it does not, stopping found the stream and cut it.
    fn open(&mut self, key: u64) -> bool {
        self.visits.remove(&key);
        !self.stopped
    }
}

/// Stops the opening once the observer has stopped.
fn running(shared: &Shared) -> Result<(), Shut> {
    match shared.active() {
        true => Ok(()),
        false => Err(Shut::Stopped),
    }
}

/// A new socket on `instance`, for every profile it serves: a ticket that
/// is asked for, used once and gone. Nothing is told to the model: a
/// socket for one answer or one snapshot failing says nothing of the
/// connection the polls run on.
pub(super) fn opened(shared: &Shared, instance: &Instance) -> Result<Socket, Shut> {
    // What the route needs for this instance is in place before the
    // ticket is asked for, since a ticket does not last. The route hears
    // of a stop while it works on that.
    running(shared)?;
    let prepared = shared.route.prepare(instance, shared);
    running(shared)?;
    prepared.map_err(Shut::Failed)?;

    // The ticket: the home it is asked in, then the control socket, once
    // more where it only was slow.
    let unheard = |why: Silent| Shut::Failed(silent(why));
    let home = home(shared);
    running(shared)?;
    let home = home.map_err(unheard)?;
    let line = ticket_line(instance, Scope::Host);
    let ask = || control::ask(&*shared.route, &home, &line, shared.pace.answer);
    let mut grant = ask();
    running(shared)?;
    if matches!(grant, Err(Silent::TimedOut)) {
        grant = ask();
        running(shared)?;
    }
    let grant = grant.map_err(unheard)?;
    let ticket = granted(&grant, instance).map_err(Shut::Denied)?;
    drop(grant);

    // The stream, listed in the step that looks at stop.
    running(shared)?;
    let dialled = Socket::dial(&*shared.route, instance);
    running(shared)?;
    let dialled = dialled.map_err(Shut::Failed)?;
    let Some(key) = shared.model().opening(dialled.line()) else {
        return Err(Shut::Stopped);
    };
    let socket = dialled.upgrade(ticket);
    if !shared.model().open(key) {
        return Err(Shut::Stopped);
    }
    socket.map_err(Shut::Failed)
}
