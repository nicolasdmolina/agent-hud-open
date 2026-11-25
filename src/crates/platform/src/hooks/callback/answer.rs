//! The answer a permission hook hears back, and whether it reached the
//! hook (Agent HUD Fork, ticket 66, round 6). An automatic Allow counts as
//! approved for the sound decision only once the reply was written to the
//! hook that asked: until then core is told nothing, so a hook that went
//! away before its answer leaves its request as any other, and it sounds.
use agent_hud_core::requests::Requests;
use serde_json::Value;

/// A hook's reply, and what core is told once it is delivered.
pub(crate) struct Answer {
    response: Value,
    /// The request it allows with no card: its source, its session and
    /// the tool call it names. None for an answer the person gave.
    approved: Option<(String, String, Option<String>)>,
}

impl Answer {
    /// The answer the person gave on the card.
    pub(super) fn by_hand(response: Value) -> Self {
        Self {
            response,
            approved: None,
        }
    }
    /// The Allow given at once, with no card, to a request of `session`.
    pub(super) fn automatic(
        response: Value,
        source: &str,
        session: &str,
        call: Option<&str>,
    ) -> Self {
        let approved = (
            source.to_owned(),
            session.to_owned(),
            call.map(str::to_owned),
        );
        Self {
            response,
            approved: Some(approved),
        }
    }
    /// Hands the reply to `send`, which says whether the hook took it,
    /// and gives it back. Only where it did is core told of the approval,
    /// where the answer is one. A listener that cannot see whether the
    /// hook took the reply says it did not: an approval nobody saw
    /// delivered is not one, and what the client then shows of that call
    /// sounds. This is the one way out of an answer, for every listener,
    /// so that no part of it is unused in any build
    /// (`windows_shape_tests.rs`).
    pub(crate) fn deliver(self, requests: &Requests, send: impl FnOnce(&Value) -> bool) -> Value {
        if let (true, Some((source, session, call))) = (send(&self.response), &self.approved) {
            requests.note_auto_approved(source, session, call.as_deref(), super::now_ms());
        }
        self.response
    }
}
