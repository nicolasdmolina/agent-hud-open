//! What the callback listener does with a callback, on either transport:
//! waits for the HUD's answer to a permission request, takes over an event
//! the callback could not write itself, and tells whether a request settles
//! in the client's own transcript.
use super::{now_ms, reminders, reviewer};
use agent_hud_agents::HookEvent;
use agent_hud_core::requests::Requests;

/// Waits for the HUD's answer to a permission request a client's hook sent,
/// while `keep_waiting` holds and until the request's deadline. None when the
/// request is not one, its reviewer is not the user, or no answer came.
pub(super) fn wait_for_answer(
    requests: &Requests,
    reminders: &reminders::Reminders,
    source: &str,
    flag: &str,
    payload: &serde_json::Value,
    keep_waiting: impl Fn() -> bool,
) -> Option<serde_json::Value> {
    let request = agent_hud_agents::parse_request(source, payload).ok()??;
    if request.requires_user_reviewer && !reviewer::is_user(requests, &request.source, payload) {
        return None;
    }
    // Approved automatically for its client: the HUD's Allow, with no card.
    // Questions and plans still wait for the user. Agent HUD Fork (ticket
    // 66): so does a call the person's own rule in the client says to ask
    // about (`RULE_ASKS`), which the HUD holds for them as the agent
    // contract says.
    let rule_asks = payload[agent_hud_agents::hooks::RULE_ASKS] == true;
    if request.kind == "permission"
        && !rule_asks
        && requests.auto_approves(&request.source, payload[super::CLIENT_KEY].as_str())
    {
        // Agent HUD Fork (ticket 66): it showed no card, so it makes no
        // sound; core is told which session it was allowed in.
        requests.note_auto_approved(
            &request.source,
            &request.session_id,
            request.call_id.as_deref(),
        );
        return request
            .response(&serde_json::json!({"kind":"allow"}))
            .ok()
            .flatten();
    }
    reminders.record(source, flag, payload);
    let ticket = requests.register(request, now_ms());
    let response = loop {
        if !keep_waiting() {
            break None;
        }
        match ticket
            .response
            .recv_timeout(std::time::Duration::from_millis(100))
        {
            Ok(response) => break response,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if requests
                    .deadline(&ticket.id)
                    .is_none_or(|at| now_ms() >= at)
                {
                    break None;
                }
            }
        }
    };
    requests.cancel(&ticket.id);
    response
}

/// The event a callback hands over the channel because it could not write
/// it to the inbox (`entry`): only one the named client's hooks report.
pub(crate) fn handed_event(source: &str, payload: &serde_json::Value) -> Option<HookEvent> {
    let event: HookEvent = serde_json::from_value(payload.get("event")?.clone()).ok()?;
    let (id, _) = agent_hud_agents::hooks::hooks_of(source).ok()?;
    let reported = matches!(
        event.kind.as_str(),
        "origin" | "attention" | "compaction" | "completion" | "session-observation"
    );
    (event.source == id && reported).then_some(event)
}

/// Whether the client's transcript shows a permission request answered in
/// its own window, so a waiting callback can stop waiting for the HUD.
pub(crate) fn settles_in_transcript(source: &str) -> bool {
    agent_hud_agents::hooks::hooks_of(source)
        .ok()
        .and_then(|(_, hooks)| hooks.permission.as_ref())
        .is_some_and(|permission| permission.settles_in_transcript)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_core::requests::AutoApprove;
    use serde_json::{json, Value};

    fn claude_asks(client: Option<&str>) -> serde_json::Value {
        let mut payload = json!({"session_id": "s1", "cwd": "/w",
            "hook_event_name": "PermissionRequest", "tool_name": "Bash",
            "tool_input": {"command": "cargo test"}});
        if let Some(client) = client {
            payload[super::super::CLIENT_KEY] = json!(client);
        }
        payload
    }

    /// Approved automatically: Claude Code hears Allow at once, with no
    /// card; a client it is off for still waits for the user.
    #[test]
    fn an_approved_client_hears_allow_at_once() {
        let requests = Requests::default();
        let reminders = reminders::Reminders::default();
        requests.set_auto_approve(AutoApprove::from_settings(&json!({
            "autoApprove": true,
            "autoApproveOff": ["claude-desktop"],
        })));
        let ask = |client| {
            wait_for_answer(
                &requests,
                &reminders,
                "claude",
                "--permission-hook",
                &claude_asks(client),
                || false,
            )
        };
        let reply = ask(Some("claude")).unwrap();
        assert_eq!(reply["hookSpecificOutput"]["decision"]["behavior"], "allow");
        // Off for Claude Desktop: no answer comes, the user is asked.
        assert!(ask(Some("claude-desktop")).is_none());
        requests.set_auto_approve(AutoApprove::default());
        assert!(ask(Some("claude")).is_none());
    }

    /// Agent HUD Fork (ticket 66): a request allowed with no card is told
    /// to core with its session and the tool call it names, so that its
    /// wait makes no sound; one that waits for the user is not.
    #[test]
    fn an_approved_request_is_noted_with_its_session_for_the_sound_decision() {
        let requests = Requests::default();
        let reminders = reminders::Reminders::default();
        let ask = |client| {
            wait_for_answer(
                &requests,
                &reminders,
                "claude",
                "--permission-hook",
                &claude_asks(client),
                || false,
            )
        };
        assert!(ask(Some("claude")).is_none());
        assert!(requests.auto_approved().is_empty());
        requests.set_auto_approve(AutoApprove::from_settings(&json!({"autoApprove": true})));
        let mut asked = claude_asks(Some("claude"));
        asked["tool_use_id"] = json!("toolu_66");
        let reply = wait_for_answer(
            &requests,
            &reminders,
            "claude",
            "--permission-hook",
            &asked,
            || false,
        );
        assert!(reply.is_some());
        let session = agent_hud_agents::parse_request("claude", &asked)
            .unwrap()
            .unwrap()
            .session_id;
        // Kept under the tool call it names, by its session.
        assert_eq!(
            requests.auto_approved(),
            std::collections::HashMap::from([(
                format!("claude:{session}"),
                vec!["call:toolu_66".to_owned()]
            )])
        );
    }

    /// A Factory Droid call, as its hook sends it before a tool runs.
    fn droid_asks(cwd: &std::path::Path, mode: &str, command: &str, risk: &str) -> Value {
        json!({"session_id": "s1", "cwd": cwd.to_str().unwrap(),
            "permission_mode": mode, "hook_event_name": "PreToolUse",
            "tool_name": "Execute",
            "tool_input": {"command": command, "riskLevel": risk}})
    }

    /// Agent HUD Fork (ticket 66, the verifier's probe): a call the
    /// person's own rule in the client says to ask about is held for
    /// them, as the agent contract says (`RULE_ASKS`). Approve
    /// automatically never allows it, though it allows the same client's
    /// other calls. Through the real policy screen, which reads Factory's
    /// settings from a home of the test's own, and the real callback.
    #[test]
    fn a_call_the_persons_own_rule_asks_about_is_never_approved_automatically() {
        let temp = tempfile::tempdir().unwrap();
        let (home, work) = (temp.path().join("home"), temp.path().join("work"));
        std::fs::create_dir_all(home.join(".factory")).unwrap(); // plain directory
        std::fs::create_dir_all(&work).unwrap(); // plain directory
        std::fs::write(
            home.join(".factory/settings.json"),
            json!({"commandDenylist": ["synthetic-deploy"]}).to_string(),
        )
        .unwrap();
        let env = std::collections::HashMap::new();
        let mark = agent_hud_agents::hooks::RULE_ASKS;
        // Droid's level alone would run this one; the rule says to ask.
        let mut ruled = droid_asks(&work, "auto-high", "synthetic-deploy now", "high");
        assert!(super::super::policy::screen(
            "droid",
            &mut ruled,
            Some(&home),
            &env
        ));
        assert_eq!(ruled[mark], true, "the screen marks what a rule asks about");
        // One no rule names, which Droid asks about by its level.
        let mut plain = droid_asks(&work, "off", "synthetic-build", "high");
        assert!(super::super::policy::screen(
            "droid",
            &mut plain,
            Some(&home),
            &env
        ));
        assert!(plain.get(mark).is_none());

        let requests = Requests::default();
        let reminders = reminders::Reminders::default();
        let held = std::cell::Cell::new(0);
        let ask = |payload: &Value| {
            held.set(0);
            wait_for_answer(
                &requests,
                &reminders,
                "droid",
                "--permission-hook",
                payload,
                || {
                    // Whether it waits as a card, at the one look taken.
                    held.set(requests.views(now_ms()).len());
                    false
                },
            )
        };
        // Approve automatically off: both wait for the person.
        assert!(ask(&ruled).is_none());
        assert_eq!(held.get(), 1);
        assert!(ask(&plain).is_none());
        assert_eq!(held.get(), 1);

        requests.set_auto_approve(AutoApprove::from_settings(&json!({"autoApprove": true})));
        assert!(
            requests.auto_approves("droid", None),
            "the rule is on for Droid"
        );
        // The plain call is allowed at once, with no card.
        let reply = ask(&plain).expect("an automatic Allow");
        assert_eq!(
            reply["hookSpecificOutput"]["permissionDecision"], "allow",
            "{reply}"
        );
        assert_eq!(held.get(), 0);
        // The call the rule asks about still waits, and hears nothing.
        let reply = ask(&ruled);
        assert!(
            reply.is_none(),
            "an explicit ask rule was allowed: {reply:?}"
        );
        assert_eq!(held.get(), 1, "it waits as a card");
        // Only the plain one was kept as allowed with no card.
        assert_eq!(requests.auto_approved().len(), 1);
    }
}
