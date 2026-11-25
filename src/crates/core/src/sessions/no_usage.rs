//! Sessions that used no AI (Agent HUD fork, ticket 70; FORK.md, "Sessions
//! that used no AI"): a session core reads from a log, of a client whose
//! sessions record every model call's tokens (`agents::ZERO_MEANS_NO_MODEL`),
//! that shows none and neither waits nor runs. A scheduled job that ran no
//! model is one. The panel's short list leaves them out when it asks, at
//! every read, and the Sessions page keeps them. A turn that ends in one
//! sounds no `finished` only where that zero is final when the turn ends
//! (`silent_at_turn_end`). A row no log backs (a hook's or a card's alone)
//! says nothing of what was used, so it is never one.
use crate::*;

/// Whether a session of `source` that shows `tokens` and is in `state` used
/// no AI. One that waits or runs never is.
pub(crate) fn used_none(source: &str, tokens: u64, state: &str) -> bool {
    tokens == 0
        && !matches!(state, "waiting" | "running")
        && agent_hud_agents::zero_means_no_model(source)
}

/// Whether a turn that ends now in the session `row` (a snapshot row) is
/// silent: its session used no AI, and its client's count is final when a
/// turn ends here. A far machine's session (`machine`) never is: its tokens
/// come after its turn's end.
pub(crate) fn silent_at_turn_end(row: &Value) -> bool {
    row["usedNoAi"] == true
        && row["machine"].is_null()
        && row["source"]
            .as_str()
            .is_some_and(agent_hud_agents::final_at_turn_end)
}

/// A row of the session list, before its page is serialized.
pub(super) enum Row<'a> {
    Log(&'a LogHead),
    Extra(Value),
}

/// With the list's `leaveOutUnused` (the panel's short list asks; the
/// Sessions page does not), takes the sessions that used no AI out of
/// `rows`, and says how many it took.
pub(super) fn leave_out(
    params: &Value,
    rows: &mut Vec<(Row<'_>, &str)>,
    family: &session_breakdown::Family,
) -> Result<usize, String> {
    let asked = params
        .get("leaveOutUnused")
        .map(|v| v.as_bool().ok_or("leaveOutUnused must be a boolean"))
        .transpose()?
        .unwrap_or(false);
    let before = rows.len();
    if asked {
        rows.retain(|(row, state)| match row {
            Row::Log(log) => {
                let tokens = log.shown_tokens(family.usage(log).0.total());
                !used_none(&log.source, tokens, state)
            }
            Row::Extra(_) => true,
        });
    }
    Ok(before - rows.len())
}
