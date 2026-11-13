//! A short visit to one session, on a socket of its own: a new ticket, a
//! new socket, `session.resume`, what the visit is for, then detach and
//! close. Hermes gives a session's snapshot, and takes an answer, only on
//! a socket subscribed to it; a subscription that stayed would keep the
//! session alive after its own client closed it. So nothing here stays
//! open, and the detach never carries `end_reason`, which would end the
//! session itself.
//!
//! Every visit is on the model's list of live visits from the moment its
//! socket opens, with the request it answers, if any. What it writes that
//! matters (the resume, an answer, a second try) goes through `write`:
//! under the model's lock the visit is checked to be still wanted and
//! marked as writing; the write itself happens with nothing locked, within
//! a short time. Stopping the observer, or closing the card, cuts the
//! socket of every visit it finds writing, before it returns. So either
//! the check fails and nothing is written, or the write is cut and its
//! frame never completes: no answer leaves afterwards.
//!
//! A subscribed socket hears the session's events. Each one that hints at
//! a change asks for a poll, as on the socket the polls run on; what waits
//! is still read by the poll alone.
use super::{
    link::{opened, Shut},
    socket::{result, Reply, Socket},
    wire::Line,
    Model, Shared,
};
use agent_hud_agents::hermes::gateway::{
    detach, hints, resume, Command, Failure, Instance, Scope, SessionRef,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// How long the gateway gets to say it detached before the socket closes
/// anyway. Closing ends the subscription too.
const PARTING: Duration = Duration::from_millis(500);

/// One live visit, as the model lists it.
pub(super) struct Held {
    line: Arc<Line>,
    /// The request it answers; None for a snapshot.
    request: Option<String>,
    /// A write of its own is under way.
    writing: bool,
}

impl Model {
    /// Cuts the socket of every visit that is writing and that `which`
    /// names by its request: an answer on its way to a request that is
    /// over, or any write at all once the observer stops.
    pub(super) fn cut_writing(&mut self, which: impl Fn(Option<&str>) -> bool) {
        for held in self.visits.values() {
            if held.writing && which(held.request.as_deref()) {
                held.line.cut();
            }
        }
    }
}

#[cfg(test)]
impl Shared {
    /// Whether any visit is writing right now, for tests of a write that
    /// the gateway does not take.
    pub(super) fn writing(&self) -> bool {
        self.model().visits.values().any(|held| held.writing)
    }
}

pub(super) struct Visit<'a> {
    shared: &'a Shared,
    socket: Socket,
    /// Its place on the model's list.
    key: u64,
    instance: Instance,
    session: SessionRef,
    request: Option<String>,
    subscription: Option<String>,
}

impl<'a> Visit<'a> {
    /// Subscribes to `session` for a snapshot and returns the visit with
    /// what the gateway gave: everything that waits in the session now.
    /// None where the observer stopped, no socket opened, the gateway
    /// refused, or it did not answer in time; the log says which.
    pub(super) fn begin(
        shared: &'a Shared,
        instance: &Instance,
        session: &SessionRef,
        wait: Duration,
    ) -> Option<(Self, Value)> {
        Self::begin_while(shared, instance, session, wait, &|| true)
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
        Self::open(shared, instance, session, None, wait, still)
    }

    /// Subscribes to answer `request`, which the visit then stands or
    /// falls with: nothing of it is written once the request has no card.
    pub(super) fn begin_for(
        shared: &'a Shared,
        instance: &Instance,
        session: &SessionRef,
        request: &str,
        wait: Duration,
    ) -> Option<(Self, Value)> {
        Self::open(shared, instance, session, Some(request), wait, &|| true)
    }

    fn open(
        shared: &'a Shared,
        instance: &Instance,
        session: &SessionRef,
        request: Option<&str>,
        wait: Duration,
        still: &dyn Fn() -> bool,
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
        socket.write_within(shared.pace.write);
        // Listed before anything is written, so a stop always finds it.
        let key = {
            let mut model = shared.model();
            model.visited += 1;
            let held = Held {
                line: socket.line(),
                request: request.map(str::to_owned),
                writing: false,
            };
            let key = model.visited;
            model.visits.insert(key, held);
            key
        };
        let mut visit = Self {
            shared,
            socket,
            key,
            instance: instance.clone(),
            session: session.clone(),
            request: request.map(str::to_owned),
            subscription: None,
        };
        if !still() {
            return None;
        }
        let asked = visit.write(|socket| socket.send(&resume(session, Scope::Host)))?;
        let Ok(Reply::Frame(reply)) = visit.reply(asked, wait) else {
            return turned("no reply".into());
        };
        match result(&reply) {
            Ok(snapshot) => {
                visit.subscription = snapshot["subscription_id"].as_str().map(str::to_owned);
                Some((visit, snapshot.clone()))
            }
            Err(refused) => turned(format!("{refused:?}")),
        }
    }

    /// Whether the visit is still wanted: the observer runs, and where it
    /// answers a request, that request still has its card on the instance
    /// that asked.
    fn wanted(&self, model: &Model) -> bool {
        match &self.request {
            Some(request) => model.current(request, &self.instance),
            None => !model.stopped,
        }
    }

    /// Writes something that matters, if the visit is still wanted. The
    /// check and the mark that it writes are one step under the model's
    /// lock; the write runs with nothing locked. Whoever ends what the
    /// visit is for meanwhile finds the mark and cuts the socket. None
    /// where nothing was written, or not all of it: the socket is cut
    /// then, and nothing more goes out on it.
    pub(super) fn write<T>(
        &mut self,
        write: impl FnOnce(&mut Socket) -> Result<T, Failure>,
    ) -> Option<T> {
        {
            let mut model = self.shared.model();
            if !self.wanted(&model) {
                return None;
            }
            if let Some(held) = model.visits.get_mut(&self.key) {
                held.writing = true;
            }
        }
        let done = write(&mut self.socket);
        if let Some(held) = self.shared.model().visits.get_mut(&self.key) {
            held.writing = false;
        }
        done.ok()
    }

    /// A subscription lost on the way is asked for again, if the visit is
    /// still wanted.
    pub(super) fn resume_again(&mut self, wait: Duration) {
        let again = resume(&self.session, Scope::Host);
        if let Some(asked) = self.write(|socket| socket.send(&again)) {
            let _ = self.reply(asked, wait);
        }
    }

    /// Waits for the reply to the call sent with `id`. Any other frame
    /// that comes meanwhile and hints at a change asks for a poll.
    pub(super) fn reply(&mut self, id: u64, wait: Duration) -> Result<Reply, Failure> {
        let shared = self.shared;
        self.socket.reply(id, wait, &mut |frame| {
            if hints(frame) {
                shared.want_poll();
            }
        })
    }

    /// Detaches and closes. Tidying up after itself is the one thing a
    /// visit still does once it is no longer wanted; a socket that was cut
    /// says nothing more at all.
    pub(super) fn end(mut self) {
        if let Some(subscription) = self.subscription.take() {
            let parting: Command = detach(&self.session, &subscription, Scope::Host);
            if let Ok(asked) = self.socket.send(&parting) {
                let _ = self.reply(asked, PARTING);
            }
        }
    }
}

impl Drop for Visit<'_> {
    /// Off the list, and closed: with a word to the gateway where the
    /// socket still stands, with none where it was cut.
    fn drop(&mut self) {
        self.shared.model().visits.remove(&self.key);
        self.socket.close();
    }
}
