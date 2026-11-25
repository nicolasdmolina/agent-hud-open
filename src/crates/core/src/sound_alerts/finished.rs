//! The sessions whose turn ends make no `finished`.
use std::collections::HashSet;

use serde_json::Value;

/// By `(source, session)`: the sub-agents, whose turn ending is a step of
/// another's turn, and the sessions that used no AI (`usedNoAi`). The
/// island's completions come from these same rows, which name a child's
/// parent or, where the log names none, say it is one (`liveFacts.subagent`).
pub(super) fn quiet(sessions: &[Value]) -> (Keys<'_>, Keys<'_>) {
    let keys = |keep: fn(&Value) -> bool| {
        sessions
            .iter()
            .filter(|session| keep(session))
            .filter_map(|session| Some((session["source"].as_str()?, session["id"].as_str()?)))
            .collect()
    };
    (
        keys(|session| !session["parentId"].is_null() || session["liveFacts"]["subagent"] == true),
        keys(|session| session["usedNoAi"] == true),
    )
}

pub(super) type Keys<'a> = HashSet<(&'a str, &'a str)>;
