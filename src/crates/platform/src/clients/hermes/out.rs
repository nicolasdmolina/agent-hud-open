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
    /// The wait core took its decision in: whose answer this record is.
    /// A card that came back can be answered again while the thread of the
    /// answer before is still on its way out, and that thread must find
    /// that what is filed is not its own.
    pub(super) taken: u64,
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
    /// queued before this may still be read. Then core is told of the
    /// mark (`Requests::mark_native`). It shows the card marked where the
    /// request still waits, and it counts the mark against a decision it
    /// has already taken, in its own locked step: such a decision is
    /// stale when it arrives, however late (`decisions.rs`). A decision
    /// that already arrived, or that waits to be picked up, is given
    /// back here: the request waits again as it was, and its answer
    /// counts as not delivered.
    pub(super) fn lost(&self, model: &mut Model, pending: &Pending, now: i64) {
        let id = pending.id();
        model.cut_writing(|request| request == Some(id.as_str()));
        self.log.line(&format!("unreachable id={id}"));
        self.requests
            .mark_native(&id, self.view(model, pending, now));
        // A decision core already handed over, which the answers' thread
        // has not picked up, is taken from it here, so that its card is
        // back at once and not at that thread's next look. This is not
        // what keeps a stale decision from being sent: one that core has
        // taken and not yet handed over is not seen here, and is stale
        // all the same when it arrives.
        let handed = model.waiters.get(&id).map(|waiter| {
            let said = waiter.response.try_recv();
            (said, waiter.wait)
        });
        match handed {
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
