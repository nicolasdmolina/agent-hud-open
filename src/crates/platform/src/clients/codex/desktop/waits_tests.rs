//! The wait Codex Desktop's channel publishes names the requests it is
//! for, by the ids core has them under (ticket 66, round 3).
use super::super::CodexDesktopObserver;
use agent_hud_agents::codex::AGENT;
use agent_hud_core::requests::{AutoApprove, Requests};
use serde_json::{json, Value};
use std::sync::Arc;

const THREAD: &str = "00000000-0000-4000-8000-000000000066";

/// A thread's state with one approval asked, waiting on `flags`.
fn state(flags: &[&str], asks: bool) -> Value {
    let ask = json!({"id":7,"method":"mcpServer/elicitation/request","params":{"threadId":THREAD,"turnId":"turn-fixture","mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","connector_id":"computer-use","persist":["session","always"],"tool_params":{"app":"com.example.Fixture","origin":"https://fixture.invalid"},"tool_name":"access_browser_origin"}}});
    let requests = if asks { vec![ask] } else { vec![] };
    json!({"cwd":"/fixture","requests":requests,"currentPermissions":{"approvalsReviewer":"user","approvalPolicy":"on-request","sandboxPolicy":{"type":"workspaceWrite"}},"turns":[],
        "threadRuntimeStatus":{"type":"active","activeFlags":flags}})
}

fn observed(requests: &Requests) -> CodexDesktopObserver {
    let temporary = tempfile::Builder::new().prefix("").tempdir().unwrap();
    let directory = temporary.path().join("w");
    std::mem::forget(temporary);
    CodexDesktopObserver::start(directory, requests.clone(), Arc::new(|| {})).unwrap()
}

/// The requests the session's wait is for, as core has them.
fn named(requests: &Requests) -> Option<Vec<String>> {
    let key = format!("{}:{THREAD}", AGENT.id);
    requests.native_wait_requests().get(&key).cloned()
}
fn waits(requests: &Requests) -> bool {
    let all = requests.native_waiting_sessions();
    all.get(AGENT.id).is_some_and(|s| s.contains(THREAD))
}

#[test]
fn a_wait_names_its_request_only_where_every_part_of_it_has_one() {
    let requests = Requests::default();
    let mut observer = observed(&requests);
    let now = crate::hooks::now_ms();
    // Waiting on an approval the channel registered: the wait is that
    // request's, under the id its card has.
    observer
        .shared
        .reconcile(THREAD, "owner", state(&["waitingOnApproval"], true));
    let cards = requests.views(now);
    let card = cards[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(named(&requests), Some(vec![card.clone()]));
    // Waiting on the person's input too, which this channel has no card
    // for: the wait is not only the approval's, and names nothing.
    let both = state(&["waitingOnApproval", "waitingOnUserInput"], true);
    observer.shared.reconcile(THREAD, "owner", both);
    assert!(waits(&requests));
    assert_eq!(named(&requests), None);
    // Waiting on an approval with none registered, or on input alone.
    for flags in [&["waitingOnApproval"][..], &["waitingOnUserInput"][..]] {
        observer
            .shared
            .reconcile(THREAD, "owner", state(flags, false));
        assert!(waits(&requests), "{flags:?}");
        assert_eq!(named(&requests), None, "{flags:?}");
    }
    // Back to the approval alone, then no wait at all.
    observer
        .shared
        .reconcile(THREAD, "owner", state(&["waitingOnApproval"], true));
    assert_eq!(named(&requests), Some(vec![card]));
    observer
        .shared
        .reconcile(THREAD, "owner", state(&[], false));
    assert!(!waits(&requests));
    assert_eq!(named(&requests), None);
    observer.stop();
}

/// With Approve automatically on, the approval is allowed at once with no
/// card, and the wait the channel then publishes names that same request:
/// the one id core keeps it under, which is how the sound decision knows
/// the wait as its.
#[test]
fn the_wait_of_an_automatically_approved_request_names_it_by_the_id_core_keeps() {
    let requests = Requests::default();
    requests.set_auto_approve(AutoApprove::from_settings(&json!({"autoApprove": true})));
    let mut observer = observed(&requests);
    observer
        .shared
        .reconcile(THREAD, "owner", state(&["waitingOnApproval"], true));
    let now = crate::hooks::now_ms();
    assert!(requests.views(now).is_empty(), "it showed a card");
    let kept = requests.auto_approved(now);
    let kept = &kept[&format!("{}:{THREAD}", AGENT.id)];
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(named(&requests).as_ref(), Some(kept));
    observer.stop();
}
