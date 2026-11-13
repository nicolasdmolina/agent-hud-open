//! What one session waits on, worked out from one reading of it and from
//! nothing else. A reading is a run of the session's events with none
//! missing, in the gateway's order, with the sets of questions open as it
//! was taken; or a snapshot, which lists what waits outright.
//!
//! Folding the events: a request waits from its request event until an
//! event says it settled. A turn that ended takes every request of that
//! turn and of the turns before it, and a turn that started takes every
//! request of the turns before it (Hermes runs one turn of a session at a
//! time and answers nothing for an earlier one), wherever in the run the
//! request event stands. So a request of a turn that is over is never shown.
use super::ask::Ask;
use super::effect::Closed;
use super::frame::event;
use serde_json::Value;
use std::collections::BTreeMap;

/// One request that waits, with how long Hermes said it waits for it.
pub(super) type Asked = (Ask, Option<i64>);

#[derive(Debug, Default)]
pub(super) struct Reading {
    /// Every request that waits in the session, in the order asked.
    pub(super) asks: Vec<Asked>,
    /// The requests the reading shows to be over, by the gateway's own id
    /// for each, with why.
    pub(super) over: BTreeMap<String, Closed>,
    /// A question or an approval waits that Agent HUD cannot show (a kind
    /// Hermes added later, or one without its text). It gets no card, and
    /// it is why the session says it waits.
    pub(super) unshown: bool,
}

impl Reading {
    pub(super) fn holds(&self, key: &str) -> bool {
        self.asks.iter().any(|(ask, _)| ask.key() == key)
    }
    /// Whether a question or an approval waits, which is what Hermes means
    /// when it says a session waits.
    pub(super) fn prompts(&self) -> bool {
        self.unshown || self.asks.iter().any(|(ask, _)| ask.generation().is_some())
    }
    /// Takes a request out as over, unless the reading already says why.
    pub(super) fn end(&mut self, key: &str, why: Closed) {
        self.asks.retain(|(ask, _)| ask.key() != key);
        self.over.entry(key.to_owned()).or_insert(why);
    }
}

/// The questions and approvals that wait after these events, which are the
/// parameters of each event as a replay carries them.
pub(super) fn fold(events: &[Value]) -> Reading {
    let mut reading = Reading::default();
    // The newest turn seen to end, and how it ended; the newest seen to start.
    let mut ended: Option<(u64, Closed)> = None;
    let mut started: Option<u64> = None;
    // The prompts that wait and cannot be shown, each with its turn.
    let mut unshown: BTreeMap<String, Option<u64>> = BTreeMap::new();
    for seen in events.iter().filter_map(event) {
        match seen.kind {
            "clarify.request" | "approval.request" => {
                let Some(asked) = Ask::prompt(seen.payload) else {
                    if let Some(prompt) = crate::text(&seen.payload["prompt_id"]) {
                        let turn = seen.payload["execution_generation"].as_u64();
                        unshown.insert(prompt.to_owned(), turn.or(seen.generation));
                    }
                    continue;
                };
                reading.over.remove(asked.0.key());
                match reading
                    .asks
                    .iter_mut()
                    .find(|(ask, _)| ask.key() == asked.0.key())
                {
                    Some(known) => *known = asked,
                    None => reading.asks.push(asked),
                }
            }
            "clarify.settled" | "approval.settled" => {
                if let Some(prompt) = crate::text(&seen.payload["prompt_id"]) {
                    reading.end(prompt, Closed::Settled);
                    unshown.remove(prompt);
                }
            }
            "request.cancel" => {
                // Why a set of questions ended. Whether one is open is read
                // from `open_requests`, never from here. Hermes's reasons
                // are an open set.
                let why = match seen.payload["reason"].as_str().unwrap_or("") {
                    "resolved" => Closed::Settled,
                    "timeout" => Closed::TimedOut,
                    "interrupted" => Closed::Interrupted,
                    other => Closed::Withdrawn(other.to_owned()),
                };
                if let Some(request) = crate::text(&seen.payload["id"]) {
                    reading.over.insert(request.to_owned(), why);
                }
            }
            "message.start" => started = started.max(seen.generation),
            "message.complete" => {
                let Some(turn) = seen.generation else {
                    continue;
                };
                if ended.as_ref().is_none_or(|(newest, _)| turn >= *newest) {
                    let why = match seen.payload["status"].as_str() {
                        Some("interrupted") => Closed::Interrupted,
                        _ => Closed::TurnEnded,
                    };
                    ended = Some((turn, why));
                }
            }
            _ => {}
        }
    }
    // Why a request of this turn is over, where a turn's end or a later
    // turn's start says so.
    let ended_by = |turn: u64| match &ended {
        Some((newest, why)) if turn <= *newest => Some(why.clone()),
        _ if started.is_some_and(|newest| turn < newest) => Some(Closed::TurnEnded),
        _ => None,
    };
    let over: Vec<(String, Closed)> = reading
        .asks
        .iter()
        .filter_map(|(ask, _)| Some((ask.key().to_owned(), ended_by(ask.generation()?)?)))
        .collect();
    for (key, why) in over {
        reading.end(&key, why);
    }
    reading.unshown = unshown
        .values()
        .any(|turn| turn.is_none_or(|turn| ended_by(turn).is_none()));
    reading
}

/// The sets of questions open in a session (`open_requests`), each with
/// what other viewers have answered so far. Hermes says no wait for one.
pub(super) fn batches(open: &Value) -> Vec<Asked> {
    open.as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let (id, method) = (entry["id"].as_str()?, entry["method"].as_str()?);
            Some((Ask::batch(id, method, &entry["params"])?, None))
        })
        .collect()
}

/// The questions and approvals a snapshot lists (`prompts`); None for a
/// snapshot that does not say.
pub(super) fn prompts(listed: &Value) -> Option<Vec<Asked>> {
    Some(listed.as_array()?.iter().filter_map(Ask::prompt).collect())
}
