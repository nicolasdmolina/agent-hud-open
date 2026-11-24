//! Which requests a thread's wait is for (Agent HUD Fork, ticket 66). The
//! channel tells core the ids it registered them under, so that core can
//! know a wait as one request's by its id, and by nothing else: an
//! automatically approved request's wait makes no sound, and a wait that
//! names no request always does.
use super::pending::Pending;
use serde_json::Value;

/// What a thread's state says it waits on: an approval, the person's input.
pub(super) fn waiting_on(state: &Value) -> (bool, bool) {
    let status = &state["threadRuntimeStatus"];
    let flags = status["activeFlags"].as_array();
    let on = |flag: &str| {
        status["type"] == "active" && flags.is_some_and(|all| all.iter().any(|f| f == flag))
    };
    (on("waitingOnApproval"), on("waitingOnUserInput"))
}

/// The ids of the requests the wait of the thread in `state` is for, from
/// the requests read out of that same state (`parsed`): its approvals
/// where it waits on one, its questions where it waits on the person's
/// input. None at all where the state's requests could not be read, or
/// where any part of the wait has no request registered for it (a kind
/// this channel shows no card for): that wait is not only theirs. What an
/// earlier state held is never named.
pub(super) fn named(state: &Value, parsed: Option<&[Pending]>) -> Vec<String> {
    let Some(parsed) = parsed else {
        return Vec::new();
    };
    let (approval, input) = waiting_on(state);
    let of = |question: bool| {
        let mine = parsed
            .iter()
            .filter(move |item| (item.view["kind"] == "question") == question);
        mine.map(|item| item.id.clone()).collect::<Vec<_>>()
    };
    let (approvals, questions) = (of(false), of(true));
    if (approval && approvals.is_empty()) || (input && questions.is_empty()) {
        return Vec::new();
    }
    let mut ids = Vec::new();
    if approval {
        ids.extend(approvals);
    }
    if input {
        ids.extend(questions);
    }
    ids.sort();
    ids
}

// The fixture stands up a Unix socket folder in place of Codex Desktop.
#[cfg(all(test, unix))]
#[path = "waits_tests.rs"]
mod tests;
