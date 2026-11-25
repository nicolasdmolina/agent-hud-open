//! Sessions that used no AI: a session core reads from a log whose client
//! has counted tokens before, that shows none, and that neither waits nor
//! runs (FORK.md, "Sessions that used no AI"). A scheduled job that ran no
//! model is one. The panel's short list leaves them out when it asks, the
//! Sessions page keeps them, and a turn of one sounds no `finished`. A client
//! that has never counted a token, and a row no log backs (a hook's or a
//! card's alone), say nothing of what was used, so none of theirs is one.
use crate::*;

/// The clients that have counted tokens for some session here: a session of
/// theirs that shows none used none.
pub(crate) fn counting(logs: &HashMap<PathBuf, LogHead>) -> HashSet<&str> {
    logs.values()
        .filter(|log| log.usage.total() > 0)
        .map(|log| log.source.as_str())
        .collect()
}

/// Whether a session of `source` that shows `tokens` and is in `state` used
/// no AI. One that waits or runs never is.
pub(crate) fn used_none(counting: &HashSet<&str>, source: &str, tokens: u64, state: &str) -> bool {
    tokens == 0 && !matches!(state, "waiting" | "running") && counting.contains(source)
}

/// A session list's `leaveOutUnused`: whether it leaves out the sessions that
/// used no AI (the panel's short list asks; the Sessions page does not).
pub(super) fn asked(params: &Value) -> Result<bool, String> {
    params
        .get("leaveOutUnused")
        .map(|v| {
            v.as_bool()
                .ok_or_else(|| "leaveOutUnused must be a boolean".into())
        })
        .transpose()
        .map(|asked| asked.unwrap_or(false))
}

/// A row of the session list, before its page is serialized.
pub(super) enum Row<'a> {
    Log(&'a LogHead),
    Extra(Value),
}

impl Row<'_> {
    /// Whether it is a session that used no AI. Only a row a log backs can be.
    pub(super) fn used_none(
        &self,
        counting: &HashSet<&str>,
        family: &session_breakdown::Family,
        state: &str,
    ) -> bool {
        let Row::Log(log) = self else { return false };
        let tokens = log.shown_tokens(family.usage(log).0.total());
        used_none(counting, &log.source, tokens, state)
    }
}
