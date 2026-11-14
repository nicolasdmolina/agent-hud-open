//! What a search in the session list finds a session's head by.
use crate::checkpoints::LogHead;

/// Whether `search`, in lower case, is in one of the words the list shows
/// for `log`: its id, client, title, model, folder, the machine it runs on,
/// or `title`, the name its usage gives it.
pub(super) fn finds(log: &LogHead, title: Option<&str>, search: &str) -> bool {
    [
        Some(log.id.as_str()),
        Some(log.source.as_str()),
        agent_hud_agents::by_id(&log.source).map(|agent| agent.name),
        // The client as the list names it: "Codex Desktop".
        crate::naming::clients::client_name(&log.source, log.client.as_deref()).as_deref(),
        Some(log.title.as_str()),
        Some(log.model.as_str()),
        log.cwd.as_deref(),
        title,
        // The far machine's label: "Mini".
        log.machine.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|field| field.to_lowercase().contains(search))
        // The model as people read it: "Opus 4.5".
        || (!log.model.is_empty()
            && crate::naming::model_names::consumer_name(&format!(
                "{}-model:{}",
                log.source, log.model
            ))
            .to_lowercase()
            .contains(search))
}
