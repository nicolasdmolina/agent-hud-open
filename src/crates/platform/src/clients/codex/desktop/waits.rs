//! Which requests a thread's wait is for (Agent HUD Fork, ticket 66). The
//! channel tells core the ids it registered them under, so that core can
//! know a wait as one request's by its id, and by nothing else: an
//! automatically approved request's wait makes no sound, and a wait that
//! names no request always does.
use super::LivePending;
use std::collections::HashMap;

/// The ids of the requests `thread`'s wait is for: its approvals where it
/// waits on one, its questions where it waits on the person's input. None
/// at all where any part of the wait has no request registered for it (a
/// kind this channel shows no card for): that wait is not only theirs.
pub(super) fn named(
    pending: &HashMap<String, LivePending>,
    thread: &str,
    approval: bool,
    input: bool,
) -> Vec<String> {
    let of = |question: bool| {
        let mine = pending.iter().filter(move |(_, live)| {
            live.item.thread == thread && (live.item.view["kind"] == "question") == question
        });
        mine.map(|(id, _)| id.clone()).collect::<Vec<_>>()
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
