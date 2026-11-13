//! A short visit to one session, on a socket of its own: a new ticket, a
//! new socket, `session.resume`, what the visit is for, then detach and
//! close. Hermes gives a session's snapshot, and takes an answer, only on
//! a socket subscribed to it; a subscription that stayed would keep the
//! session alive after its own client closed it. So nothing here stays
//! open, and the detach never carries `end_reason`, which would end the
//! session itself.
use super::{
    link::{opened, Shut},
    socket::{result, Reply, Socket},
    Shared,
};
use agent_hud_agents::hermes::gateway::{detach, resume, Instance, Scope, SessionRef};
use serde_json::Value;
use std::time::Duration;

/// How long the gateway gets to say it detached before the socket closes
/// anyway. Closing ends the subscription too.
const PARTING: Duration = Duration::from_millis(500);

pub(super) struct Visit {
    pub(super) socket: Socket,
    session: SessionRef,
    subscription: Option<String>,
}

impl Visit {
    /// Subscribes to `session` and returns the visit with the snapshot the
    /// gateway gave: everything that waits in the session now. None where
    /// no socket opened, the gateway refused, or it did not answer in time;
    /// the log says which.
    pub(super) fn begin(
        shared: &Shared,
        instance: &Instance,
        session: &SessionRef,
        wait: Duration,
    ) -> Option<(Self, Value)> {
        let turned = |why: String| -> Option<(Self, Value)> {
            let line = format!("visit failed session={} why={why}", session.hud_id());
            shared.log.line(&line);
            None
        };
        let mut socket = match opened(shared, instance) {
            Ok(socket) => socket,
            Err(Shut::Denied(denied)) => return turned(format!("{denied:?}")),
            Err(Shut::Failed(failure)) => return turned(format!("{failure:?}")),
        };
        let reply = socket.call(&resume(session, Scope::Host), wait, &mut |_| {});
        let Ok(Reply::Frame(reply)) = reply else {
            socket.close();
            return turned("no reply".into());
        };
        match result(&reply) {
            Ok(snapshot) => {
                let visit = Self {
                    socket,
                    session: session.clone(),
                    subscription: snapshot["subscription_id"].as_str().map(str::to_owned),
                };
                Some((visit, snapshot.clone()))
            }
            Err(refused) => {
                socket.close();
                turned(format!("{refused:?}"))
            }
        }
    }

    /// Detaches and closes.
    pub(super) fn end(mut self) {
        if let Some(subscription) = &self.subscription {
            let parting = detach(&self.session, subscription, Scope::Host);
            let _ = self.socket.call(&parting, PARTING, &mut |_| {});
        }
        self.socket.close();
    }
}
