//! An answer given here between its first byte and its receipt, and what
//! a card closing says of it.
use super::{Model, Shared};
use agent_hud_agents::hermes::gateway::Closed;

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
