//! What becomes of an answer given here: the model hears that it was
//! written, and then what Hermes replied to it.
use super::ask::Ask;
use super::effect::{Closed, Effect};
use super::frame::{answered, Answered, Refusal};
use super::state::{Card, Gateway, Key};
use serde_json::Value;

impl Gateway {
    fn key(&self, request: &str) -> Option<Key> {
        let is = |card: &&Card| card.pending.id() == request;
        let pending = &self.cards.values().find(is)?.pending;
        Some((pending.session.clone(), pending.ask.key().to_owned()))
    }

    /// An answer was written. Nothing replies to the answer to a set of
    /// questions, so only a poll tells what became of it.
    pub(super) fn sent(&mut self, request: &str, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(card) = self.key(request).and_then(|key| self.cards.get_mut(&key)) else {
            return;
        };
        card.sent_ms = Some(now_ms);
        if matches!(card.pending.ask, Ask::Batch(_)) {
            effects.push(Effect::Poll);
        }
    }

    /// Hermes replied to an answer. Resolved, already resolved and a turn
    /// that is over all mean the request is over: the card closes, a poll
    /// begun before the reply is dropped (it may still say the request
    /// waits), and a new one is asked for at once.
    pub(super) fn responded(&mut self, request: &str, reply: &Value, effects: &mut Vec<Effect>) {
        let Some(key) = self.key(request) else {
            return;
        };
        let why = match answered(reply) {
            Answered::Resolved => Closed::Answered,
            Answered::AlreadyResolved => Closed::AlreadyResolved,
            Answered::Refused(Refusal::StaleGeneration) => Closed::StaleGeneration,
            Answered::Refused(refusal) => {
                if let Some(card) = self.cards.get_mut(&key) {
                    card.sent_ms = None;
                    effects.push(Effect::Unanswered {
                        pending: card.pending.clone(),
                        why: Some(refusal),
                    });
                }
                return;
            }
        };
        self.polls += 1;
        self.close(&key, why, effects);
        effects.push(Effect::Poll);
    }
}
