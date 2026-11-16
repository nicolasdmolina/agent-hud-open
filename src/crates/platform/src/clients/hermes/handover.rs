//! What the followers of a machine do when the way to it changes. Cards,
//! readers and followers belong to the machine and not to the entry that
//! leads to it, so a route change ends none of them: an entry's address
//! was edited, or the entry that owned the machine was removed while
//! another reaches it (`machines/book.rs`).
//!
//! Three steps, each for every follower of the machine:
//!
//! - `lose`, taken in the step that raises the route number
//!   (`route/switch.rs`). Each follower takes it as a lost socket, with
//!   its model locked: its cards are marked in place, an answer being
//!   written is cut, a decision core took and has not handed over is
//!   stale, and a poll that is under way counts for nothing when it
//!   comes back. Nothing is settled and nothing is registered again, so
//!   a card keeps the time it came, the hold that runs from it, and
//!   whether it was dismissed.
//! - `relabel`, when the machine goes by another label: its owner was
//!   renamed, or another entry owns it now. Every card that shows is
//!   changed in place.
//! - `stir`, once the new way is in: every follower dials at once, and
//!   the first poll finds what still waits and takes its mark away.
use super::{followers::Followers, Shared};
use agent_hud_agents::hermes::gateway::{Failure, Input, Pending};
use std::sync::PoisonError;

impl Followers {
    /// The way to the machine is gone, as of now.
    pub(super) fn lose(&self) {
        for follower in self.set().iter() {
            follower.lose();
        }
    }

    /// The machine's cards say `label` from now on.
    pub(super) fn relabel(&self, label: &str) {
        // Its stored sessions too: the mirrors' writer writes the label.
        if let Some(mirror) = &self.mirror {
            mirror.relabel(label);
        }
        let followers = self.set();
        {
            let Some(first) = followers.first() else {
                return;
            };
            let mut said = first.label.lock().unwrap_or_else(PoisonError::into_inner);
            if said.as_deref() == Some(label) {
                return;
            }
            *said = Some(label.to_owned());
        }
        for follower in followers.iter() {
            follower.relabel();
        }
    }

    /// There is a way to the machine again: every follower asks now, and
    /// the machine is looked at whole.
    pub(super) fn stir(&self) {
        for follower in self.set().iter() {
            follower.stir();
        }
        self.look.want();
    }
}

impl Shared {
    /// The socket counts as lost from this step on, whatever becomes of
    /// it: the thread that holds it finds the model no longer ready at
    /// its next poll, and dials again.
    fn lose(&self) {
        self.apply(Input::Lost(Failure::Unreachable));
        self.want_poll();
    }

    /// Shows every card again as it is now, under the request it has.
    fn relabel(&self) {
        let model = self.model();
        if self.ending(&model) {
            return;
        }
        let now = crate::hooks::now_ms();
        let waiting: Vec<Pending> = model.gateway.waiting().cloned().collect();
        for pending in waiting {
            let view = self.view(&model, &pending, now);
            self.requests.refresh_native(&pending.id(), view);
        }
        drop(model);
        (self.wake)();
    }
}
