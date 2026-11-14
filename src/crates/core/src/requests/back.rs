//! Giving a request back. `answer` takes a native request's card away
//! before its adapter has sent the decision anywhere. Where the adapter
//! then cannot send it (its client went out of reach in between), the
//! request still waits in the client, and the adapter gives it back: the
//! same request as it was, not a new one. A request that came to its end
//! meanwhile is given back to nobody.
//!
//! Which decisions are not to be sent is kept here too, where no order of
//! threads can get it wrong. A request counts how often it was marked as
//! out of reach, and a decision is taken at the count of that moment, in
//! the one locked step that takes the card away. A mark counts on, in a
//! locked step of its own, whether the request still shows or its decision
//! is already taken. So a decision is stale exactly when its request was
//! marked after it was taken, however long the decision then took to
//! reach its adapter, and whatever became of the mark since.
use super::{Content, Pending, Requests, State, Ticket};
use serde_json::Value;
use std::sync::{mpsc, PoisonError};

/// A native request a decision was taken for: which of its waits the
/// decision was taken in, and when the request first came, which its hold
/// runs from.
pub(super) struct Taken {
    pub(super) wait: u64,
    pub(super) created_ms: i64,
    /// The request's mark count when the decision was taken.
    pub(super) epoch: u64,
    /// Its mark count now: marks go on counting while it is taken.
    pub(super) marks: u64,
    /// The decision was handed to its adapter (`hand_over`).
    pub(super) handed: bool,
}

impl State {
    /// Numbers the next wait: one for each registration and each put-back.
    pub(super) fn begin_wait(&mut self) -> u64 {
        self.waits = self.waits.wrapping_add(1);
        self.waits
    }
}

impl Requests {
    /// Puts back the native request `id`, whose decision was taken in
    /// `wait` (its ticket's) and could not be sent: it waits again as it
    /// was, under the same id, with the time it first came and the hold
    /// that runs from it, and not dismissed. `view` is its card as it is
    /// now, which may say why it takes no answer (`unreachable`). The new
    /// ticket is where its next decision arrives.
    ///
    /// Nothing is put back, and None returned, where the request came to
    /// its end since the decision was taken: its hold ran out, its source
    /// settled or cancelled it, everything was cleared, or it was put back
    /// already. A put-back never brings back what a later change ended,
    /// and never starts a hold anew.
    pub fn put_back(&self, id: &str, wait: u64, mut view: Value, now_ms: i64) -> Option<Ticket> {
        // What ran out by now is over, taken or not.
        self.expire(now_ms);
        let (sender, response) = mpsc::channel();
        let ticket = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.taken.get(id)?.wait != wait || state.pending.iter().any(|p| p.id == id) {
                return None;
            }
            let Taken {
                created_ms, marks, ..
            } = state.taken.remove(id)?;
            let deadline_ms = created_ms.saturating_add(state.hold_ms as i64);
            state.dismissed.remove(id);
            view["id"] = Value::String(id.into());
            let wait = state.begin_wait();
            // Back where it stood: the requests are in the order they came.
            let later = |p: &Pending| p.created_ms > created_ms;
            let at = state.pending.iter().position(later);
            let at = at.unwrap_or(state.pending.len());
            state.pending.insert(
                at,
                Pending {
                    id: id.into(),
                    content: Content::Native(view),
                    created_ms,
                    deadline_ms,
                    sender,
                    wait,
                    marks,
                },
            );
            state.revision = state.revision.wrapping_add(1);
            Ticket {
                id: id.into(),
                response,
                deadline_ms,
                wait,
            }
        };
        self.wake();
        Some(ticket)
    }

    /// Marks the native request `id` as out of reach: its client cannot be
    /// reached from now on. `view` is its card saying so (`unreachable`),
    /// shown in place of the one it has where the request still waits, as
    /// `refresh_native` shows one. Where a decision was already taken for
    /// it, nothing shows, and that decision is stale from this step on
    /// (`current`): the mark counts for a request that waits and for one
    /// whose decision is on its way alike.
    pub fn mark_native(&self, id: &str, mut view: Value) {
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(taken) = state.taken.get_mut(id) {
                taken.marks = taken.marks.wrapping_add(1);
            }
            view["id"] = Value::String(id.into());
            let waits = state.pending.iter_mut().find(|p| p.id == id);
            let changed = waits.is_some_and(|item| {
                item.marks = item.marks.wrapping_add(1);
                match &mut item.content {
                    Content::Native(shown) if *shown != view => {
                        *shown = view;
                        true
                    }
                    _ => false,
                }
            });
            if changed {
                state.revision = state.revision.wrapping_add(1);
            }
            changed
        };
        if changed {
            self.wake();
        }
    }

    /// Whether the decision taken in `wait` for the native request `id`
    /// is still one to send: it is the decision that is out for the
    /// request, and the request was not marked as out of reach since it
    /// was taken. An adapter asks when the decision reaches it, which can
    /// be a while after it was taken. A decision that is not current is
    /// not sent; its request can be put back (`put_back`) for a new one.
    pub fn current(&self, id: &str, wait: u64) -> bool {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let taken = state.taken.get(id);
        taken.is_some_and(|taken| taken.wait == wait && taken.epoch == taken.marks)
    }

    /// Hands the decision `answer` took from `item` to the adapter that
    /// waits for it. `answer` takes a decision under the lock and comes
    /// here after letting go of it, so the request can come to its end in
    /// between. A decision its source took back with the request
    /// (`close_native`) is not handed over: it is over, its source has
    /// said what became of it, and this does nothing. Whether it was
    /// taken back is looked at, and the decision handed over, in one
    /// locked step, so the two can never both happen. Whether the
    /// decision is delivered, or needs no delivery any more.
    pub(super) fn hand_over(&self, id: &str, item: &Pending, response: Option<Value>) -> bool {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.back.remove(&item.wait) {
            return true;
        }
        let taken = state.taken.get_mut(id);
        if let Some(taken) = taken.filter(|taken| taken.wait == item.wait) {
            taken.handed = true;
        }
        item.sender.send(response).is_ok()
    }

    /// Ends the native request `id`, which its source says is over, as
    /// `settle_native` does, and takes back the decision that was taken
    /// for it, if one is still on record: one whose adapter has not given
    /// the request back. Returns the wait that decision was taken in.
    /// All in one locked step: no decision can be taken for the request
    /// after it, and one that was taken and not yet handed over never
    /// will be (`hand_over`). So the adapter learns of every decision
    /// core took for a request it closes, whether or not the decision
    /// reached it, and can say once what became of it.
    pub fn close_native(&self, id: &str) -> Option<u64> {
        let (item, taken) = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let taken = state.taken.remove(id);
            if let Some(taken) = taken.as_ref().filter(|taken| !taken.handed) {
                state.back.insert(taken.wait);
            }
            let waits = state.pending.iter().position(|p| p.id == id);
            let item = waits.and_then(|index| state.pending.remove(index));
            state.dismissed.remove(id);
            if item.is_some() {
                state.revision = state.revision.wrapping_add(1);
            }
            (item, taken.map(|taken| taken.wait))
        };
        if let Some(item) = item {
            let _ = item.sender.send(None);
            self.wake();
        }
        taken
    }
}
