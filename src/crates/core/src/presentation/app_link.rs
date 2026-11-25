//! A client's app link, standing in as the way back of a session that has
//! none of its own: offered only where the host can open it, and called
//! what it does where its client names it (Hermes's "Open in Hermes
//! desktop", ticket 39).
use crate::ProviderDiscovery;
use serde_json::{json, Value};

/// Gives `session` of `source` its client's app link, where the session
/// has no way back yet and the host found one it can open.
pub(super) fn stand_in(discovery: &[ProviderDiscovery], source: &str, session: &mut Value) {
    if !session["navigationTarget"].is_null() {
        return;
    }
    let Some(target) = (discovery.iter())
        .find(|found| found.id == source)
        .and_then(|found| found.navigation_target.as_ref())
    else {
        return;
    };
    session["navigationTarget"] = target.clone();
    let named = agent_hud_agents::by_id(source)
        .and_then(|agent| agent.discovery.app_navigation)
        .filter(|link| target["kind"] == link.kind)
        .and_then(|link| link.label);
    if let Some(label) = named {
        session["navigationLabel"] = json!(label());
    }
}
