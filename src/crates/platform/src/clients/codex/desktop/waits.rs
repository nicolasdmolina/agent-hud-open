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

/// The ids of the requests the wait of `thread` in `state` is for, from
/// the requests read out of that same state (`parsed`). They are named
/// only where the wait is known to be theirs and nothing else's:
///
/// - the state's requests could be read (what an earlier state held is
///   never named);
/// - it waits on an approval and not on the person's input, which no
///   request stands for that could have been allowed for them;
/// - every request the state lists as waiting in the thread is one this
///   channel shows a card for. One it skips (a connector it does not know,
///   a shape it cannot read) still waits for the person, so a wait that
///   holds one is not only the others'.
///
/// Anything else names none, and the sound decision lets such a wait sound.
pub(super) fn named(state: &Value, thread: &str, parsed: Option<&[Pending]>) -> Vec<String> {
    let Some(parsed) = parsed else {
        return Vec::new();
    };
    let (approval, input) = waiting_on(state);
    let shown = parsed.iter().filter(|item| item.view["kind"] != "question");
    let mut ids: Vec<String> = shown.map(|item| item.id.clone()).collect();
    // A request that is done, or another thread's, does not wait here.
    let waits_here = |request: &&Value| {
        let of = request["params"]["threadId"].as_str();
        request["completed"] != true && of.is_none_or(|id| id == thread)
    };
    let listed = state["requests"].as_array();
    let waiting = listed.map_or(0, |all| all.iter().filter(waits_here).count());
    if !approval || input || ids.is_empty() || waiting != ids.len() {
        return Vec::new();
    }
    ids.sort();
    ids
}

// The fixture stands up a Unix socket folder in place of Codex Desktop.
#[cfg(all(test, unix))]
#[path = "waits_tests.rs"]
mod tests;
