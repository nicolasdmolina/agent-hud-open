//! The decisions core hands over. Core takes a decision made on a card,
//! puts the card away, and hands the decision to the waiter the card was
//! registered with. Each is looked at here as it arrives, and one that
//! may still go is sent on a thread of its own (`answer.rs`).
//!
//! A decision can arrive a while after core took it. If its card was
//! marked as out of reach in between, it is stale, and stays stale
//! whatever became of the mark since: core says so (`Requests::current`),
//! from what it counted under its own lock when it took the decision and
//! when the card was marked. A stale decision is never sent. Its card is
//! given back to core as it was, for a new answer.
use super::{
    answer::{unreachable, Going},
    visit::Visit,
    Shared,
};
use agent_hud_agents::hermes::gateway::{Pending, Reply, Scope};
use serde_json::Value;
use std::{
    sync::{mpsc::TryRecvError, Arc},
    thread,
    time::Duration,
};

/// How often the waiters are looked at: an answer leaves within this.
const GLANCE: Duration = Duration::from_millis(50);
/// Takes each decision core hands over and sends it on a thread of its
/// own, so one slow answer never holds up another.
pub(super) fn watch(shared: Arc<Shared>) {
    while shared.active() {
        // A test may keep the decisions waiting where core left them.
        #[cfg(test)]
        if shared.deaf.load(std::sync::atomic::Ordering::Acquire) {
            shared.pause(GLANCE);
            continue;
        }
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

/// The decisions that arrived and may still go, each with its request as
/// it waits now and the wait core took it in, which is filed as the
/// answer that is out. A waiter core let go of (the card was left alone,
/// or its time in the HUD ran out) is forgotten; the request goes on
/// waiting in Hermes.
///
/// A decision whose card was marked as out of reach after core took it
/// is not among them, whether or not the mark is still there: core
/// counted both under its own lock, so no order in which the decision
/// and the mark reached this thread can make it look current. Its card
/// is given back as it was, for a new answer, and it counts as not
/// delivered.
fn decided(shared: &Shared) -> Vec<(Pending, Value, u64)> {
    let mut model = shared.model();
    // A follower that is ending takes no decision: the end has what
    // waits for it, and says nothing of it.
    if shared.ending(&model) {
        return Vec::new();
    }
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
    let now = crate::hooks::now_ms();
    arrived.retain(|(pending, _, wait)| {
        let id = pending.id();
        model.taken.insert(id.clone(), *wait);
        let current = shared.requests.current(&id, *wait);
        if !current {
            shared.log.line(&format!("undelivered id={id}"));
            shared.back(&mut model, pending, now);
            let session = pending.session.hud_id();
            shared.note(&id, &session, "failed", Some(unreachable()), now);
        }
        current
    });
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
    let Some(instance) = shared.answering(&id, taken) else {
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
