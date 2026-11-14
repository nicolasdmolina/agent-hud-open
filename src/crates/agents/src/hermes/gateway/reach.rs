//! Cards whose gateway cannot be reached. An outage ends no question:
//! Hermes may go on waiting, so the card stays, and nothing is withdrawn
//! for being out of reach. But there is nowhere to send an answer, so the
//! card is marked, and a marked card takes none. Nothing here has a timer:
//! how long a card is held, and how long Hermes waits, are as they were.
//!
//! A card is marked when the socket to its gateway is lost, and when an
//! answer to it could not be sent. The mark goes when a poll that counted
//! read its session whole and found the request waiting: the first poll
//! after the connection is back does that for every request that still
//! waits, and closes the ones that are over. A poll that could not tell
//! (a list or a read that failed) leaves the mark.
use super::effect::Effect;
use super::state::{Card, Gateway};

impl Card {
    /// The gateway is out of reach: the card says so, once.
    fn out_of_reach(&mut self, effects: &mut Vec<Effect>) {
        if !std::mem::replace(&mut self.marked, true) {
            effects.push(Effect::Marked(self.pending.clone()));
        }
    }

    /// A poll found the request waiting: the card is as it was, once.
    pub(super) fn in_reach(&mut self, effects: &mut Vec<Effect>) {
        if std::mem::replace(&mut self.marked, false) {
            effects.push(Effect::Unmarked(self.pending.clone()));
        }
    }
}

impl Gateway {
    /// Whether the card of the request with this id is marked as out of
    /// reach. No answer is sent to a marked request.
    pub fn marked(&self, request: &str) -> bool {
        let named = |card: &&Card| card.pending.id() == request;
        self.cards
            .values()
            .find(named)
            .is_some_and(|card| card.marked)
    }

    /// The connection is gone: every card is marked.
    pub(super) fn out_of_reach(&mut self, effects: &mut Vec<Effect>) {
        for card in self.cards.values_mut() {
            card.out_of_reach(effects);
        }
    }

    /// An answer to `request` could not be sent. Whatever the connection
    /// looks like, the gateway did not take an answer just now: the card
    /// is marked until a poll finds the request waiting again, and one is
    /// asked for at once.
    pub(super) fn unsent(&mut self, request: &str, effects: &mut Vec<Effect>) {
        let named = |card: &&mut Card| card.pending.id() == request;
        if let Some(card) = self.cards.values_mut().find(named) {
            card.out_of_reach(effects);
            effects.push(Effect::Poll);
        }
    }
}
