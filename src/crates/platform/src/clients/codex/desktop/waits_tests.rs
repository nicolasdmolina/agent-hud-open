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

/// The real channel beside a whole core, so that a state Codex Desktop
/// streams is followed to the snapshot's session row, cards and sounds
/// (the round 3 verifier's fixture).
struct Replay {
    core: agent_hud_core::AgentHudCore,
    observer: CodexDesktopObserver,
    _temp: tempfile::TempDir,
}

impl Replay {
    /// A Codex Desktop session at work, sound and Approve automatically on.
    fn new() -> Self {
        use agent_hud_core::{AgentHudCore, SourceRoot};
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("sessions");
        std::fs::create_dir(&folder).unwrap(); // plain directory
        let at = (chrono::Utc::now() - chrono::Duration::seconds(2)).to_rfc3339();
        let events = [
            json!({"timestamp":at,"type":"session_meta","payload":{"id":THREAD,"timestamp":at,"source":"vscode","originator":"Codex Desktop","cwd":"/fixture"}}),
            json!({"timestamp":at,"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-fixture"}}),
        ];
        let log: String = events.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(folder.join(format!("rollout-{THREAD}.jsonl")), log).unwrap();
        let roots = vec![SourceRoot {
            path: folder,
            provider: "codex",
        }];
        let mut core = AgentHudCore::new(roots, temp.path().join("app"));
        for key in ["sound", "autoApprove"] {
            core.request("set_setting", json!({"key": key, "value": true}))
                .unwrap();
        }
        core.request("refresh", json!({})).unwrap();
        let observer =
            CodexDesktopObserver::start(temp.path().join("w"), core.requests(), Arc::new(|| {}))
                .unwrap();
        let mut replay = Self {
            core,
            observer,
            _temp: temp,
        };
        assert_eq!(replay.look(), ("running".to_owned(), 0, 0));
        replay
    }
    fn streams(&self, state: Value) {
        self.observer.shared.reconcile(THREAD, "owner", state);
    }
    /// One look: (the session's state, cards shown, `needs-you` alerts).
    fn look(&mut self) -> (String, usize, usize) {
        let snapshot = self.core.request("request_snapshot", json!({})).unwrap();
        let rows = snapshot["sessions"].as_array().unwrap();
        let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
        let state = row["state"].as_str().unwrap().to_owned();
        let cards = snapshot["approvals"].as_array().unwrap().len();
        let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
        let needs = alerts.iter().filter(|item| item["kind"] == "needs-you");
        (state, cards, needs.count())
    }
}

/// The round 3 verifier's second probe. A thread's approval was allowed
/// automatically and its wait passed in silence. The next state still says
/// it waits on an approval and gives no list of requests: the channel has
/// nothing to say which request the wait is for now. It names none, never
/// the request it knew before, and the wait sounds.
#[test]
fn a_wait_whose_requests_cannot_be_read_names_none_and_sounds() {
    for broken in ["no list", "a list that is not one"] {
        let mut replay = Replay::new();
        let requests = replay.core.requests();
        replay.streams(state(&["waitingOnApproval"], true));
        assert!(named(&requests).is_some());
        assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0));
        let mut unknown = state(&["waitingOnApproval"], false);
        match broken {
            "no list" => drop(unknown.as_object_mut().unwrap().remove("requests")),
            _ => unknown["requests"] = json!("unreadable"),
        }
        replay.streams(unknown);
        assert_eq!(named(&requests), None, "{broken}: an old request named");
        assert_eq!(replay.look(), ("waiting".to_owned(), 0, 1), "{broken}");
    }
}

/// A second request beside the fixture's, differing in what `change` sets.
fn another(base: &Value, id: u64, change: impl FnOnce(&mut Value)) -> Value {
    let mut request = base["requests"][0].clone();
    request["id"] = json!(id);
    change(&mut request);
    request
}

/// The round 3 verifier's first probe. Two approvals wait in one thread
/// under one flag. The channel can show the first, which is allowed
/// automatically; the second is for a connector it has no card for, and
/// needs the person. The wait is not the first one's alone, so it names
/// no request and sounds, as the second one alone does.
#[test]
fn a_wait_with_an_approval_the_channel_cannot_show_names_none_and_sounds() {
    let base = state(&["waitingOnApproval"], true);
    let unshown = another(&base, 8, |request| {
        request["params"]["_meta"]["connector_id"] = json!("other-connector");
    });
    let mut replay = Replay::new();
    let requests = replay.core.requests();
    let mut both = base.clone();
    both["requests"]
        .as_array_mut()
        .unwrap()
        .push(unshown.clone());
    replay.streams(both);
    let kept = requests.auto_approved(crate::hooks::now_ms());
    assert_eq!(
        kept[&format!("{}:{THREAD}", AGENT.id)].len(),
        1,
        "the first"
    );
    assert_eq!(named(&requests), None, "a wait named for part of itself");
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 1));

    // The control: the one the channel cannot show, alone.
    let mut control = Replay::new();
    let mut alone = base.clone();
    alone["requests"] = json!([unshown]);
    control.streams(alone);
    assert_eq!(control.look(), ("waiting".to_owned(), 0, 1));
}

/// What does not make a wait more than its requests': a second approval
/// the channel shows too, a request that is already completed, and one of
/// another thread. Each wait names every approval that waits in its
/// thread, all of them allowed, and is silent.
#[test]
fn a_wait_names_its_requests_where_every_approval_that_waits_in_the_thread_is_one() {
    let base = state(&["waitingOnApproval"], true);
    let cases: [(&str, Value, usize); 3] = [
        ("a second one shown", another(&base, 8, |_| {}), 2),
        (
            "one completed",
            another(&base, 8, |request| {
                request["completed"] = json!(true);
                request["params"]["_meta"]["connector_id"] = json!("other-connector");
            }),
            1,
        ),
        (
            "one of another thread",
            another(&base, 8, |request| {
                request["params"]["threadId"] = json!("00000000-0000-4000-8000-000000000067");
                request["params"]["_meta"]["connector_id"] = json!("other-connector");
            }),
            1,
        ),
    ];
    for (case, beside, count) in cases {
        let mut replay = Replay::new();
        let requests = replay.core.requests();
        let mut both = base.clone();
        both["requests"].as_array_mut().unwrap().push(beside);
        replay.streams(both);
        assert_eq!(named(&requests).map(|ids| ids.len()), Some(count), "{case}");
        assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0), "{case}");
    }
}
