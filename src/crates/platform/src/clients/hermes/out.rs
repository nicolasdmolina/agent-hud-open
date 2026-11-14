//! An answer given here between its first byte and its receipt, and what
//! a card closing, or its gateway going out of reach, says of it.
use super::{answer::unreachable, effects::unconfirmed, Model, Shared};
use agent_hud_agents::hermes::gateway::{Closed, Instance, Pending};
use std::sync::mpsc::TryRecvError;

/// An answer given here that is out, or on its way out: what its receipt
/// needs. It is filed in the model in the very step that marks its visit
/// as writing, before the first byte, so that whatever closes the card
/// finds it, whichever comes first: the card closing, the write returning,
/// or Hermes's reply.
pub(super) struct Out {
    /// A set of questions: nothing replies to its answer.
    pub(super) batch: bool,
    /// All of its frame is written.
    pub(super) written: bool,
    /// Its card closed before what became of the answer was known, and why.
    pub(super) closed: Option<Closed>,
}

impl Shared {
    /// The card of `id` closed. An answer given here gets its receipt now
    /// where the close itself says what became of it: Hermes's reply closed
    /// it, or it is a set of questions whose frame is out, which nothing
    /// replies to. Otherwise the answer is still on its way, or its reply
    /// is: the close is kept with it, and the thread that sends it gives
    /// the receipt when it knows.
    pub(super) fn closed(
        &self,
        model: &mut Model,
        id: &str,
        session: &str,
        why: &Closed,
        now: i64,
    ) {
        let Some(out) = model.answered.get_mut(id) else {
            return;
        };
        let replied = matches!(
            why,
            Closed::Answered | Closed::AlreadyResolved | Closed::StaleGeneration
        );
        if replied || (out.batch && out.written) {
            let batch = out.batch;
            model.answered.remove(id);
            self.receipt(id, session, why, batch, now);
        } else {
            out.closed = Some(why.clone());
        }
    }
}

impl Shared {
    /// The instance to send the answer taken in `taken` to, while that
    /// answer is still the one that is out for `request`: one that was
    /// given back since is nobody's, whatever the gateway is now.
    pub(super) fn answering(&self, request: &str, taken: u64) -> Option<Instance> {
        let model = self.model();
        let out = model.taken.get(request) == Some(&taken);
        model.instance_of(request).filter(|_| out)
    }

    /// The card of `pending` is out of reach from now on: its socket was
    /// lost, or an answer to it could not be sent. That ends every
    /// decision core took for it before: none is sent later, also not
    /// once the gateway is back, when the card takes a new answer.
    ///
    /// An answer being written to it is cut, as when a card closes
    /// (`visit.rs`): a write that was let through and has not begun never
    /// begins, and one under way never completes. A frame that was fully
    /// queued before this may still be read. Then the card is shown
    /// marked, from which moment core takes no decision for it; so a
    /// decision found waiting after that is all there was. That one, and
    /// one the answers' thread already holds, is given back: the request
    /// waits again as it was, and its answer counts as not delivered.
    pub(super) fn lost(&self, model: &mut Model, pending: &Pending, now: i64) {
        let id = pending.id();
        model.cut_writing(|request| request == Some(id.as_str()));
        self.reach(model, pending, "unreachable", now);
        let decided = model.waiters.get(&id).map(|waiter| {
            let said = waiter.response.try_recv();
            (said, waiter.wait)
        });
        match decided {
            Some((Ok(Some(_)), wait)) => {
                model.waiters.remove(&id);
                model.taken.insert(id.clone(), wait);
            }
            Some((Ok(None) | Err(TryRecvError::Disconnected), _)) => {
                model.waiters.remove(&id);
            }
            Some((Err(TryRecvError::Empty), _)) | None => {}
        }
        if !model.taken.contains_key(&id) {
            return;
        }
        // Filed means its frame was let through: some of it may be out.
        let (state, why) = match model.answered.contains_key(&id) {
            true => ("uncertain", unconfirmed()),
            false => ("failed", unreachable()),
        };
        self.log.line(&format!("undelivered id={id}"));
        self.back(model, pending, now);
        self.note(&id, &pending.session.hud_id(), state, Some(why), now);
    }
}
