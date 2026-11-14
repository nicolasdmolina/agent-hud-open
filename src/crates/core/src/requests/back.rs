//! Giving a request back. `answer` takes a native request's card away
//! before its adapter has sent the decision anywhere. Where the adapter
//! then cannot send it (its client went out of reach in between), the
//! request still waits in the client, and the adapter gives it back: the
//! same request as it was, not a new one. A request that came to its end
//! meanwhile is given back to nobody.
use super::{Content, Pending, Requests, State, Ticket};
use serde_json::Value;
use std::sync::{mpsc, PoisonError};

/// A native request a decision was taken for: which of its waits the
/// decision was taken in, and when the request first came, which its hold
/// runs from.
pub(super) struct Taken {
    pub(super) wait: u64,
    pub(super) created_ms: i64,
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
            let created_ms = state.taken.remove(id)?.created_ms;
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
}
