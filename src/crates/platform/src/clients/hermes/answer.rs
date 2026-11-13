//! Answers. Core hands the decision made on a card to the waiter the card
//! was registered with, and puts the card away. The decision becomes the
//! frame the model builds for it and goes to Hermes on a visit of its own:
//! subscribe, send, hear the reply, detach, close. The model is told the
//! answer was written and then what Hermes replied; it closes the card or
//! brings it back.
use super::{
    socket::{result, Reply as Came},
    visit::Visit,
    Shared,
};
use agent_hud_agents::hermes::gateway::{
    refusal, resume, Ask, Command, Input, Pending, Refusal, Reply, Scope,
};
use agent_hud_core::l10n;
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
        for (pending, decision) in decided(&shared) {
            let owned = shared.clone();
            let sent = pending.clone();
            let spawned = thread::Builder::new()
                .name("Hermes answer".into())
                .spawn(move || deliver(&owned, sent, &decision));
            if spawned.is_err() {
                shared.undelivered(&pending, unreachable());
            }
        }
        shared.pause(GLANCE);
    }
}

/// The decisions that arrived, each with its request as it waits now. A
/// waiter core let go of (the card was left alone, or its time in the HUD
/// ran out) is forgotten; the request goes on waiting in Hermes.
fn decided(shared: &Shared) -> Vec<(Pending, Value)> {
    let mut model = shared.model();
    let (mut arrived, mut over) = (Vec::new(), Vec::new());
    for (id, waiter) in &model.waiters {
        match waiter.response.try_recv() {
            Ok(Some(decision)) => {
                over.push(id.clone());
                if let Some(pending) = model.gateway.pending(id) {
                    arrived.push((pending.clone(), decision));
                }
            }
            Ok(None) | Err(TryRecvError::Disconnected) => over.push(id.clone()),
            Err(TryRecvError::Empty) => {}
        }
    }
    for id in over {
        model.waiters.remove(&id);
    }
    arrived
}

fn deliver(shared: &Shared, pending: Pending, decision: &Value) {
    let reply = match pending.reply(decision, Scope::Host) {
        Ok(Some(reply)) => reply,
        // Nothing to send: the request is left to Hermes's own windows, or
        // every question of a set was skipped, which leaves the set to its
        // other viewers. That is done, not an answer that failed.
        Ok(None) => return shared.log.line(&format!("answer left id={}", pending.id())),
        Err(reason) => return shared.undelivered(&pending, reason),
    };
    let id = pending.id();
    // Nothing is sent for a card that closed meanwhile; one that still
    // shows with no gateway to send to comes back saying so.
    let Some(instance) = shared.instance_of(&id) else {
        return shared.undelivered(&pending, unreachable());
    };
    let wait = shared.pace.answer * 2;
    // An answer from a socket that is not subscribed is refused.
    let Some((mut visit, _)) = Visit::begin(shared, &instance, &pending.session, wait) else {
        return shared.undelivered(&pending, unreachable());
    };
    match reply {
        // Nothing replies to the answer to a set of questions: a poll
        // tells what became of it.
        Reply::Result(frame) => {
            if visit.socket.write(&frame).is_ok() {
                sent(shared, &pending);
            } else {
                shared.undelivered(&pending, unreachable());
            }
        }
        Reply::Call(command) => {
            let mut told = call(shared, &mut visit, &pending, &command, wait);
            if refusal(&told).is_some_and(|refused| refused.retryable()) && shared.active() {
                shared.pause(AGAIN);
                if refusal(&told) == Some(Refusal::PermissionDenied) {
                    let again = resume(&pending.session, Scope::Host);
                    let _ = visit.socket.call(&again, wait, &mut |_| {});
                }
                told = call(shared, &mut visit, &pending, &command, wait);
            }
            shared.apply(Input::Responded {
                request: &id,
                reply: &told,
            });
        }
    }
    visit.end();
}

/// Sends the answer, tells the model it was written, and returns the reply
/// frame; where none came, a frame that says so.
fn call(
    shared: &Shared,
    visit: &mut Visit,
    pending: &Pending,
    command: &Command,
    wait: Duration,
) -> Value {
    let silent = || json!({"error": {"message": NO_REPLY, "data": {"reason": NO_REPLY}}});
    let Ok(id) = visit.socket.send(command) else {
        return silent();
    };
    sent(shared, pending);
    match visit.socket.reply(id, wait, &mut |_| {}) {
        Ok(Came::Frame(reply)) if result(&reply).is_ok() || refusal(&reply).is_some() => reply,
        _ => silent(),
    }
}

/// The answer was written: the model hears it, and the card is remembered
/// as answered here so its receipt can be given when it closes.
fn sent(shared: &Shared, pending: &Pending) {
    let id = pending.id();
    let mut model = shared.model();
    if model.stopped {
        return;
    }
    shared.log.line(&format!("answer sent id={id}"));
    let batch = matches!(pending.ask, Ask::Batch(_));
    model.answered.insert(id.clone(), batch);
    shared.take(&mut model, Input::Sent { request: &id });
}

fn unreachable() -> String {
    l10n::text(
        "无法连接 Hermes 来送出这个回答",
        "Hermes could not be reached to send the answer.",
    )
}
