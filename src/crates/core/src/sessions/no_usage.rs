//! Sessions that used no AI (Agent HUD fork, ticket 70; FORK.md, "Sessions
//! that used no AI"): a session core reads from a log, of a client listed in
//! `agents::ZERO_MEANS_NO_MODEL`, whose store says it holds no reply of a
//! model (`LogHead::replied`), that shows no tokens and neither waits nor
//! runs. A scheduled job that ran no model is one. The panel's short list
//! leaves them out when it asks, at every read, so a reply or tokens read
//! later bring a session back; the Sessions page keeps them. Sounds are not
//! touched. A session whose store says nothing of replies (a far machine's
//! mirror, every other client) and a row no log backs are never one.
use crate::*;

/// Whether the session `log`, showing `tokens` and in `state`, used no AI.
/// One that waits or runs never is.
pub(crate) fn used_none(log: &LogHead, tokens: u64, state: &str) -> bool {
    tokens == 0
        && log.replied == Some(false)
        && !matches!(state, "waiting" | "running")
        && agent_hud_agents::zero_means_no_model(&log.source)
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
                !used_none(log, tokens, state)
            }
            Row::Extra(_) => true,
        });
    }
    Ok(before - rows.len())
}
