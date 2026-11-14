//! Answers. Core hands the decision made on a card to the waiter the card
//! was registered with, and puts the card away. The decision becomes the
//! frame the model builds for it and goes to Hermes on a visit of its own:
//! subscribe, send, hear the reply, detach, close. The model is told the
//! answer was written and then what Hermes replied; it closes the card or
//! brings it back.
//!
//! Core refuses a decision for a card that is marked as out of reach, and
//! the same rule holds here for a decision core took a moment before the
//! mark: no socket is opened for it, nothing is written, and the card is
//! given back to core as it was (`effects.rs`, `undelivered`).
use super::{
    effects::too_late,
    out::Out,
    socket::{result, Reply as Came, Socket},
    visit::Visit,
    Model, Shared,
};
use agent_hud_agents::hermes::gateway::{
    answered, refusal, Answered, Ask, Closed, Command, Failure, Input, Pending, Refusal, Reply,
    Scope,
};
use agent_hud_core::l10n;
#[cfg(test)]
use probes::{at_apply, before_reply};
#[cfg(test)]
pub(super) use probes::{verifier3_written, AT_APPLY, BEFORE_REPLY};
use serde_json::{json, Value};
use std::{
    sync::{mpsc::TryRecvError, Arc},
    thread,
    time::Duration,
};

/// How often the waiters are looked at: an answer leaves within this.
const GLANCE: Duration = Duration::from_millis(50);
/// How long before an answer Hermes could not take just now is sent again.
const AGAIN: Duration = Duration::from_millis(300);
/// The reason given to the model where Hermes sent no reply to an answer.
pub(super) const NO_REPLY: &str = "no_reply";

/// Takes each decision core hands over and sends it on a thread of its
/// own, so one slow answer never holds up another.
pub(super) fn watch(shared: Arc<Shared>) {
    while shared.active() {
        for (pending, decision, wait) in decided(&shared) {
            let owned = shared.clone();
            let sent = pending.clone();
            let spawned = thread::Builder::new()
                .name("Hermes answer".into())
                .spawn(move || deliver(&owned, sent, &decision, wait));
            if spawned.is_err() {
                shared.undelivered(&pending, wait, unreachable());
            }
        }
        shared.pause(GLANCE);
    }
}

/// The decisions that arrived, each with its request as it waits now and
/// the wait core took it in, which is filed as the answer that is out. A
/// waiter core let go of (the card was left alone, or its time in the HUD
/// ran out) is forgotten; the request goes on waiting in Hermes.
fn decided(shared: &Shared) -> Vec<(Pending, Value, u64)> {
    let mut model = shared.model();
    let (mut arrived, mut over) = (Vec::new(), Vec::new());
    for (id, waiter) in &model.waiters {
        match waiter.response.try_recv() {
            Ok(Some(decision)) => {
                over.push(id.clone());
                if let Some(pending) = model.gateway.pending(id) {
                    arrived.push((pending.clone(), decision, waiter.wait));
                }
            }
            Ok(None) | Err(TryRecvError::Disconnected) => over.push(id.clone()),
            Err(TryRecvError::Empty) => {}
        }
    }
    for id in over {
        model.waiters.remove(&id);
    }
    for (pending, _, wait) in &arrived {
        model.taken.insert(pending.id(), *wait);
    }
    arrived
}

fn deliver(shared: &Shared, pending: Pending, decision: &Value, taken: u64) {
    let reply = match pending.reply(decision, Scope::Host) {
        Ok(Some(reply)) => reply,
        // Nothing to send: the request is left to Hermes's own windows.
        // That is done, not an answer that failed.
        Ok(None) => return shared.log.line(&format!("answer left id={}", pending.id())),
        Err(reason) => return shared.undelivered(&pending, taken, reason),
    };
    let id = pending.id();
    // Nothing is sent for a card that closed meanwhile. One that still
    // shows with no gateway to send to, or that was marked as out of
    // reach since core took the decision, opens no socket and is put back.
    let Some(instance) = shared.instance_of(&id) else {
        return shared.undelivered(&pending, taken, unreachable());
    };
    let wait = shared.pace.answer * 2;
    // An answer from a socket that is not subscribed is refused.
    let visit = Visit::begin_for(shared, &instance, &pending.session, &id, wait);
    let Some((mut visit, _)) = visit else {
        return shared.undelivered(&pending, taken, unreachable());
    };
    let going = Going {
        shared,
        pending: &pending,
        taken,
    };
    let told = match reply {
        // Nothing replies to the answer to a set of questions: a poll
        // tells what became of it.
        Reply::Result(frame) => going
            .written(&mut visit, |socket| socket.write(&frame))
            .map(|()| None),
        Reply::Call(command) => going.asked(&mut visit, &command, wait).map(Some),
    };
    match told {
        Some(Some(told)) => going.replied(&told),
        Some(None) => {}
        // Nothing was written. A card that closed, or an observer that
        // stopped, is left alone; a card that still shows comes back.
        None => shared.undelivered(&pending, taken, unreachable()),
    }
    visit.end();
}

/// One answer on its way, and what it answers.
struct Going<'a> {
    shared: &'a Shared,
    pending: &'a Pending,
    /// The wait core took the decision in. The answer is the one that is
    /// out only while the model files that wait for its card.
    taken: u64,
}

impl Going<'_> {
    /// Writes one frame of the answer, if the answer may still go: the
    /// observer runs, the request still has its card, the gateway is still
    /// the instance that asked (`Visit::write_marked`, which checks under
    /// the model's lock and writes outside it; a stop, or the card closing,
    /// cuts the socket of a write under way). The answer is filed as on its
    /// way in that same locked step, and only while it is the answer that
    /// is out: one whose card came back since writes nothing more. Once
    /// the frame is out the model hears it was written; a frame that did
    /// not get out is forgotten. None where it was not written, or not
    /// all of it.
    fn written<T>(
        &self,
        visit: &mut Visit,
        write: impl FnOnce(&mut Socket) -> Result<T, Failure>,
    ) -> Option<T> {
        let id = self.pending.id();
        let batch = matches!(self.pending.ask, Ask::Batch(_));
        let file = |model: &mut Model| {
            if model.taken.get(&id) != Some(&self.taken) {
                return false;
            }
            let out = Out {
                batch,
                written: false,
                closed: None,
            };
            model.answered.insert(id.clone(), out);
            true
        };
        let done = visit.write_marked(file, write);
        let mut model = self.shared.model();
        let Some(done) = done else {
            // Not checked through, cut, or out of time: no answer left.
            if model.answered.get(&id).is_some_and(|out| !out.written) {
                model.answered.remove(&id);
            }
            return None;
        };
        let now = crate::hooks::now_ms();
        let session = self.pending.session.hud_id();
        match model.answered.remove(&id) {
            // The card closed while the frame went out. For a set of
            // questions that is the whole receipt; a question or an
            // approval still has Hermes's reply to come.
            Some(out) if out.closed.is_some() && out.batch => {
                let why = out.closed.unwrap_or(Closed::Gone);
                self.shared.receipt(&id, &session, &why, true, now);
            }
            Some(mut out) => {
                out.written = true;
                let open = out.closed.is_none();
                model.answered.insert(id.clone(), out);
                if open && !model.stopped && model.gateway.pending(&id).is_some() {
                    self.shared.log.line(&format!("answer sent id={id}"));
                    self.shared.take(&mut model, Input::Sent { request: &id });
                }
            }
            None => {}
        }
        Some(done)
    }

    /// Hermes replied to the answer, or did not in time. Where a poll
    /// closed the card before the reply came, the reply is all that says
    /// what became of the answer, and gives its receipt; else the model
    /// hears it. Both under one hold of the model's lock: a poll that
    /// closes the card comes wholly before this, and the reply gives the
    /// receipt, or wholly after, and finds the card already closed by the
    /// reply. It cannot land between looking and acting.
    fn replied(&self, told: &Value) {
        let id = self.pending.id();
        #[cfg(test)]
        before_reply(self.shared, &id);
        let mut model = self.shared.model();
        let closed = model
            .answered
            .get(&id)
            .is_some_and(|out| out.closed.is_some());
        // A follower that ended takes nothing more. One that ended with
        // its gateway (`follower.rs`) kept the answers whose cards it
        // closed, and the reply still gives each its receipt.
        if model.stopped && !closed {
            return;
        }
        // The card of this answer came back since (its socket was lost
        // under it) and may have been answered again: a refusal of this
        // one says nothing of the request as it waits now.
        let stale = model.taken.get(&id) != Some(&self.taken);
        let refused =
            |told| matches!(told, Answered::Refused(why) if why != Refusal::StaleGeneration);
        if !closed && stale && refused(answered(told)) {
            return;
        }
        if !closed {
            let heard = Input::Responded {
                request: &id,
                reply: told,
            };
            #[cfg(test)]
            at_apply(self.shared, &id);
            return self.shared.take(&mut model, heard);
        }
        model.answered.remove(&id);
        let (session, now) = (self.pending.session.hud_id(), crate::hooks::now_ms());
        match answered(told) {
            Answered::Resolved => self.shared.note(&id, &session, "confirmed", None, now),
            Answered::AlreadyResolved | Answered::Refused(Refusal::StaleGeneration) => self
                .shared
                .note(&id, &session, "failed", Some(too_late()), now),
            Answered::Refused(_) => {}
        }
    }

    /// Sends the answer and returns the reply frame; where none came, a
    /// frame that says so. An answer Hermes could not take just now is
    /// sent once more, while it may still go. None where it never left.
    fn asked(&self, visit: &mut Visit, command: &Command, wait: Duration) -> Option<Value> {
        let told = self.heard(visit, command, wait)?;
        if !refusal(&told).is_some_and(|refused| refused.retryable()) {
            return Some(told);
        }
        self.shared.pause(AGAIN);
        if refusal(&told) == Some(Refusal::PermissionDenied) {
            // Not subscribed after all: subscribe again, if it still matters.
            visit.resume_again(wait);
        }
        // What the first try was told stands where the second never left.
        Some(self.heard(visit, command, wait).unwrap_or(told))
    }

    /// One try: the write, then the reply.
    fn heard(&self, visit: &mut Visit, command: &Command, wait: Duration) -> Option<Value> {
        let silent = || json!({"error": {"message": NO_REPLY, "data": {"reason": NO_REPLY}}});
        let id = self.written(visit, |socket| socket.send(command))?;
        Some(match visit.reply(id, wait) {
            Ok(Came::Frame(reply)) if result(&reply).is_ok() || refusal(&reply).is_some() => reply,
            _ => silent(),
        })
    }
}

fn unreachable() -> String {
    l10n::text(
        "无法连接 Hermes 来送出这个回答",
        "Hermes could not be reached to send the answer.",
    )
}

/// What only the tests use: a way into `Going::written`, and the places
/// in the reply handler a test may be told of.
#[cfg(test)]
mod probes {
    use super::*;

    // Test-only entry to the unchanged production Going::written path.
    pub(in super::super) fn verifier3_written<T>(
        shared: &Shared,
        pending: &Pending,
        visit: &mut Visit,
        write: impl FnOnce(&mut Socket) -> Result<T, Failure>,
    ) -> Option<T> {
        // The decision core handed over is filed as `decided` files it.
        let taken = 0;
        shared.model().taken.insert(pending.id(), taken);
        let going = Going {
            shared,
            pending,
            taken,
        };
        going.written(visit, write)
    }

    /// Where a test may pause an answer's thread: after Hermes's reply is read
    /// and before it is handled, which is where a poll can still close the
    /// card. Set by the tests that need it; nothing in the app calls it. It is
    /// told which observer the answer belongs to (by address) as well as the
    /// request, since tests that run side by side replay the same recordings
    /// and so share request ids.
    pub(in super::super) static BEFORE_REPLY: Probe = Probe::new(None);

    /// Where a test may look at an answer's thread: the card was found open
    /// and the reply is about to be given to the model, which has to be still
    /// held from the look at the card. A test looks from another thread
    /// whether it is.
    pub(in super::super) static AT_APPLY: Probe = Probe::new(None);

    /// A place a test may be told of: the observer, by address, and the request.
    pub(in super::super) type Probe = std::sync::RwLock<Option<fn(usize, &str)>>;

    pub(super) fn before_reply(shared: &Shared, id: &str) {
        told(&BEFORE_REPLY, shared, id);
    }

    pub(super) fn at_apply(shared: &Shared, id: &str) {
        told(&AT_APPLY, shared, id);
    }

    fn told(probe: &Probe, shared: &Shared, id: &str) {
        let test = *probe.read().unwrap_or_else(|held| held.into_inner());
        if let Some(test) = test {
            test(shared as *const Shared as usize, id);
        }
    }
}
