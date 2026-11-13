//! A short visit to one session, on a socket of its own: a new ticket, a
//! new socket, `session.resume`, what the visit is for, then detach and
//! close. Hermes gives a session's snapshot, and takes an answer, only on
//! a socket subscribed to it; a subscription that stayed would keep the
//! session alive after its own client closed it. So nothing here stays
//! open, and the detach never carries `end_reason`, which would end the
//! session itself.
//!
//! A subscribed socket hears the session's events. Each one that hints at
//! a change asks for a poll, as on the socket the polls run on; what waits
//! is still read by the poll alone.
use super::{
    link::{opened, Shut},
    socket::{result, Reply, Socket},
    Shared,
};
use agent_hud_agents::hermes::gateway::{
    detach, hints, resume, Command, Failure, Instance, Scope, SessionRef,
};
use serde_json::Value;
use std::time::Duration;

/// How long the gateway gets to say it detached before the socket closes
/// anyway. Closing ends the subscription too.
const PARTING: Duration = Duration::from_millis(500);

pub(super) struct Visit<'a> {
    shared: &'a Shared,
    pub(super) socket: Socket,
    session: SessionRef,
    subscription: Option<String>,
}

impl<'a> Visit<'a> {
    /// Subscribes to `session` and returns the visit with the snapshot the
    /// gateway gave: everything that waits in the session now. None where
    /// the observer stopped, no socket opened, the gateway refused, or it
    /// did not answer in time; the log says which.
    pub(super) fn begin(
        shared: &'a Shared,
        instance: &Instance,
        session: &SessionRef,
        wait: Duration,
    ) -> Option<(Self, Value)> {
        Self::begin_while(shared, instance, session, wait, &|| shared.active())
    }

    /// The same, subscribing only while `still` holds once the socket is
    /// open: getting a ticket and dialling take time, and what the visit
    /// is for may be over by then.
    pub(super) fn begin_while(
        shared: &'a Shared,
        instance: &Instance,
        session: &SessionRef,
        wait: Duration,
        still: &dyn Fn() -> bool,
    ) -> Option<(Self, Value)> {
        let turned = |why: String| -> Option<(Self, Value)> {
            let line = format!("visit failed session={} why={why}", session.hud_id());
            shared.log.line(&line);
            None
        };
        let socket = match opened(shared, instance) {
            Ok(socket) => socket,
            Err(Shut::Denied(denied)) => return turned(format!("{denied:?}")),
            Err(Shut::Failed(failure)) => return turned(format!("{failure:?}")),
        };
        let mut visit = Self {
            shared,
            socket,
            session: session.clone(),
            subscription: None,
        };
        if !still() {
            visit.socket.close();
            return None;
        }
        let reply = visit.call(&resume(session, Scope::Host), wait);
        let Ok(Reply::Frame(reply)) = reply else {
            visit.socket.close();
            return turned("no reply".into());
        };
        match result(&reply) {
            Ok(snapshot) => {
                visit.subscription = snapshot["subscription_id"].as_str().map(str::to_owned);
                Some((visit, snapshot.clone()))
            }
            Err(refused) => {
                visit.socket.close();
                turned(format!("{refused:?}"))
            }
        }
    }

    /// Makes one call and waits for its reply. Any other frame that comes
    /// meanwhile and hints at a change asks for a poll.
    pub(super) fn call(&mut self, command: &Command, wait: Duration) -> Result<Reply, Failure> {
        let id = self.socket.send(command)?;
        self.reply(id, wait)
    }

    /// Waits for the reply to the call sent with `id`, hearing hints.
    pub(super) fn reply(&mut self, id: u64, wait: Duration) -> Result<Reply, Failure> {
        let shared = self.shared;
        self.socket.reply(id, wait, &mut |frame| {
            if hints(frame) {
                shared.want_poll();
            }
        })
    }

    /// Detaches and closes.
    pub(super) fn end(mut self) {
        if let Some(subscription) = self.subscription.take() {
            let parting = detach(&self.session, &subscription, Scope::Host);
            let _ = self.call(&parting, PARTING);
        }
        self.socket.close();
    }
}
