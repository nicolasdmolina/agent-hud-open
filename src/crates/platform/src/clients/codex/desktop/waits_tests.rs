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
    // Not approved until its answer is confirmed, by the wait it was
    // given in; the wait the channel publishes names it by its id.
    assert!(requests.auto_approved(now).is_empty());
    let ids = named(&requests).unwrap();
    assert_eq!(ids.len(), 1, "{ids:?}");
    let wait = {
        let state = observer.shared.state.lock().unwrap();
        state.pending[&ids[0]].ticket.as_ref().unwrap().wait
    };
    requests.auto_answer_delivered(wait, now);
    let kept = requests.auto_approved(now);
    assert_eq!(kept[&format!("{}:{THREAD}", AGENT.id)], ids);
    // Stopping the listener settles the request: nothing is kept of it.
    observer.stop();
    assert!(requests.auto_approved(now).is_empty());
}

/// The real channel beside a whole core, so that a state Codex Desktop
/// streams is followed to the snapshot's session row, cards and sounds
/// (the round 3 verifier's fixture).
struct Replay {
    core: agent_hud_core::AgentHudCore,
    observer: CodexDesktopObserver,
    /// Where Codex Desktop would listen: a socket that takes the channel's
    /// connection and never answers. The channel's worker waits there for
    /// seconds, so it neither sends a queued answer nor drops what the
    /// test streams in because it found nobody to connect to.
    _owner: std::os::unix::net::UnixListener,
    temp: tempfile::TempDir,
}

impl Replay {
    /// A Codex Desktop session at work, sound and Approve automatically on.
    fn new() -> Self {
        use agent_hud_core::{AgentHudCore, SourceRoot};
        let temp = tempfile::Builder::new().prefix("").tempdir().unwrap();
        let folder = temp.path().join("sessions");
        std::fs::create_dir(&folder).unwrap(); // plain directory
        std::fs::create_dir_all(temp.path().join("w/ipc")).unwrap(); // plain directory
        let socket = temp.path().join("w/ipc/ipc.sock");
        let owner = std::os::unix::net::UnixListener::bind(socket).unwrap();
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
            _owner: owner,
            temp,
        };
        assert_eq!(replay.look(), ("running".to_owned(), 0, 0));
        replay
    }
    /// Stops the listener and opens another on the same core and socket.
    fn reopens(&mut self) {
        self.observer.stop();
        let data = self.temp.path().join("w");
        let requests = self.core.requests();
        self.observer = CodexDesktopObserver::start(data, requests, Arc::new(|| {})).unwrap();
    }
    fn streams(&self, state: Value) {
        self.observer.shared.reconcile(THREAD, "owner", state);
    }
    /// The owner confirms the automatic answer to every request the wait
    /// names, as the channel's reply worker tells it (`reply.rs`; this
    /// fixture's socket never answers): by the wait each answer was given
    /// in, with the receipt beside it. Only then are they approved.
    fn owner_confirms(&self) {
        let requests = self.core.requests();
        let now = crate::hooks::now_ms();
        for id in named(&requests).unwrap_or_default() {
            let state = self.observer.shared.state.lock().unwrap();
            let wait = state.pending[&id].ticket.as_ref().map(|ticket| ticket.wait);
            drop(state);
            if let Some(wait) = wait {
                requests.auto_answer_delivered(wait, now);
            }
            requests.note_native_delivery(&id, AGENT.id, THREAD, "confirmed", None, now);
        }
    }
    /// One look: (the session's state, cards shown, `needs-you` alerts).
    fn look(&mut self) -> (String, usize, usize) {
        let snapshot = self.core.request("request_snapshot", json!({})).unwrap();
        let rows = snapshot["sessions"].as_array().unwrap();
        let row = rows.iter().find(|row| row["id"] == THREAD).unwrap();
        let state = row["state"].as_str().unwrap().to_owned();
        let cards = snapshot["approvals"].as_array().unwrap().len();
        let alerts = snapshot["soundAlerts"]["items"].as_array().unwrap();
        // As the host plays them: each alert once, by its id.
        let needs = alerts.iter().filter(|item| item["kind"] == "needs-you");
        let ids: std::collections::HashSet<&str> =
            needs.filter_map(|item| item["id"].as_str()).collect();
        (state, cards, ids.len())
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
        replay.owner_confirms();
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
    // The first was allowed with no card; the second shows none either.
    assert!(requests.views(crate::hooks::now_ms()).is_empty());
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
        replay.owner_confirms();
        assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0), "{case}");
    }
}

/// The round 5 verifier's second probe, through the real channel. Codex
/// Desktop streams a pending permission, which is allowed automatically,
/// and the listener stops before the queued answer could leave (this
/// fixture has no socket for it to leave by). Approve automatically is
/// turned off, a listener opens again on the same core, and the owner's
/// unchanged frame is replayed: the first time the app looks at the
/// request. It shows its card, and the card sounds, once. Before round 6
/// the card showed with no sound.
#[test]
fn a_card_that_returns_after_the_listener_stopped_before_its_answer_left_sounds() {
    let mut replay = Replay::new();
    let requests = replay.core.requests();
    let wire = state(&["waitingOnApproval"], true);
    replay.streams(wire.clone());
    {
        let state = replay.observer.shared.state.lock().unwrap();
        let queued = state.pending.values().filter(|live| live.ticket.is_some());
        assert_eq!(queued.count(), 1, "the automatic answer is queued");
    }
    replay.observer.stop();
    let now = crate::hooks::now_ms();
    assert!(requests.deliveries(now).is_empty(), "the answer left");
    assert!(requests.auto_approved(now).is_empty());

    let off = json!({"key": "autoApprove", "value": false});
    replay.core.request("set_setting", off).unwrap();
    replay.reopens();
    replay.streams(wire);
    assert_eq!(replay.look(), ("waiting".to_owned(), 1, 1));
    assert_eq!(replay.look(), ("waiting".to_owned(), 1, 1));
    replay.observer.stop();
}

/// The same where the app had looked before the listener stopped, and
/// does not look again until the card is back: the wait of the
/// unconfirmed request sounded then, and the card that returns is that
/// same wait. One sound in all, as before the sync. (A look taken while
/// the listener is stopped sees the wait end, and the card is then a new
/// wait: that is the probe above.)
#[test]
fn a_card_that_returns_after_its_unconfirmed_wait_sounded_is_the_same_wait() {
    let mut replay = Replay::new();
    let wire = state(&["waitingOnApproval"], true);
    replay.streams(wire.clone());
    let off = json!({"key": "autoApprove", "value": false});
    replay.core.request("set_setting", off).unwrap();
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 1));
    replay.reopens();
    replay.streams(wire);
    assert_eq!(replay.look(), ("waiting".to_owned(), 1, 1));
    replay.observer.stop();
}

/// The round 6 verifier's replacement probe, through the real channel. One
/// approval waits, the app looks, and in the next frame that approval is
/// done and another one waits: both of a kind the channel shows, both
/// under `waitingOnApproval`, with no frame between that says the thread
/// is at work. They are two requests, so two sounds, as before the sync,
/// where each showed its card. With Approve automatically on neither
/// answer has left (the stand-in owner never answers), so neither is
/// approved, and the second one's wait must sound though the first one's
/// did. A failed receipt for the second then adds nothing: the same wait.
/// Before round 7 the second approval waited with no sound.
#[test]
fn a_second_approval_that_takes_the_place_of_the_first_sounds_too() {
    for automatic in [true, false] {
        let mut replay = Replay::new();
        let requests = replay.core.requests();
        let auto = json!({"key": "autoApprove", "value": automatic});
        replay.core.request("set_setting", auto).unwrap();
        let cards = usize::from(!automatic);
        let first = state(&["waitingOnApproval"], true);
        let mut second = first.clone();
        let mut done = first["requests"][0].clone();
        done["completed"] = json!(true);
        second["requests"] = json!([done, another(&first, 8, |_| {})]);

        replay.streams(first);
        let seventh = named(&requests).unwrap();
        assert_eq!(
            replay.look(),
            ("waiting".to_owned(), cards, 1),
            "{automatic}"
        );
        replay.streams(second);
        let eighth = named(&requests).unwrap();
        assert_eq!((seventh.len(), eighth.len()), (1, 1));
        assert_ne!(seventh, eighth, "the two approvals go by one id");
        assert!(requests.deliveries(crate::hooks::now_ms()).is_empty());
        assert_eq!(
            replay.look(),
            ("waiting".to_owned(), cards, 2),
            "automatic: {automatic}; the second approval made no sound"
        );
        assert_eq!(replay.look(), ("waiting".to_owned(), cards, 2));
        if automatic {
            let now = crate::hooks::now_ms();
            let why = Some("fixture refusal".to_owned());
            requests.note_native_delivery(&eighth[0], AGENT.id, THREAD, "failed", why, now);
            assert_eq!(
                replay.look(),
                ("waiting".to_owned(), 0, 2),
                "it sounded again"
            );
        }
        replay.observer.stop();
    }
}

/// The same pair where the first answer was confirmed before the app
/// looked: its wait is silent, and the second approval, not confirmed, is
/// the first sound.
#[test]
fn a_second_approval_after_a_confirmed_one_sounds() {
    let mut replay = Replay::new();
    let first = state(&["waitingOnApproval"], true);
    let mut second = first.clone();
    let mut done = first["requests"][0].clone();
    done["completed"] = json!(true);
    second["requests"] = json!([done, another(&first, 8, |_| {})]);
    replay.streams(first);
    replay.owner_confirms();
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0));
    replay.streams(second);
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 1));
    replay.owner_confirms();
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 1));
    replay.observer.stop();
}

/// Round 9, every way the channel loses sight of a thread's requests
/// without the stand-in sockets: the stream drops (`forget_connection`),
/// the thread becomes unknown to it (`cards::unknown`), and a state comes
/// whose requests cannot be read. Each time the approval was confirmed
/// and its wait had passed in silence. What the channel shows next under
/// the same id is registered anew, with an answer of its own to send, and
/// is not approved until that one is confirmed: it sounds.
#[test]
fn a_request_seen_again_after_the_channel_lost_sight_of_it_sounds_and_is_answered_anew() {
    for lost in [
        "the stream drops",
        "the thread is unknown",
        "its requests cannot be read",
    ] {
        for looks_between in [false, true] {
            let about = format!("{lost}, a look between: {looks_between}");
            let mut replay = Replay::new();
            let requests = replay.core.requests();
            let wire = state(&["waitingOnApproval"], true);
            replay.streams(wire.clone());
            let id = named(&requests).unwrap().remove(0);
            let ticket = |replay: &Replay| {
                let state = replay.observer.shared.state.lock().unwrap();
                state.pending[&id].ticket.as_ref().map(|ticket| ticket.wait)
            };
            let first = ticket(&replay).expect("the first answer is queued");
            replay.owner_confirms();
            assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0), "{about}");
            // What stands for a wait that names nothing, where there is one.
            let mut sounds = 0;
            match lost {
                "the stream drops" => replay.observer.shared.forget_connection(),
                "the thread is unknown" => {
                    super::super::cards::unknown(&replay.observer.shared, THREAD)
                }
                _ => {
                    let mut unreadable = wire.clone();
                    unreadable["requests"] = json!("unreadable");
                    replay.streams(unreadable);
                }
            }
            assert!(ticket(&replay).is_none(), "{about}");
            let now = crate::hooks::now_ms();
            assert!(
                requests.auto_approved(now).is_empty(),
                "{about}: still approved"
            );
            if looks_between {
                // A state that cannot be read still says the thread waits:
                // a wait that names nothing, which sounds.
                let waits = lost == "its requests cannot be read";
                sounds += usize::from(waits);
                let state = if waits { "waiting" } else { "running" };
                assert_eq!(replay.look(), (state.to_owned(), 0, sounds), "{about}");
            }
            replay.streams(wire);
            let second = ticket(&replay).expect("it is answered anew");
            assert_ne!(first, second, "{about}");
            assert!(requests.auto_approved(now).is_empty(), "{about}");
            sounds += 1;
            assert_eq!(replay.look(), ("waiting".to_owned(), 0, sounds), "{about}");
            // Word of the first answer, late, confirms nothing of it.
            requests.auto_answer_delivered(first, now);
            assert!(requests.auto_approved(now).is_empty(), "{about}");
            replay.owner_confirms();
            assert_eq!(requests.auto_approved(now).len(), 1, "{about}");
            assert_eq!(replay.look(), ("waiting".to_owned(), 0, sounds), "{about}");
            replay.observer.stop();
        }
    }
}

/// The control for the three: a request the channel never lost sight of
/// is not registered again by the next state that lists it, and stays
/// approved and silent.
#[test]
fn a_request_the_channel_kept_in_sight_is_not_answered_twice() {
    let mut replay = Replay::new();
    let requests = replay.core.requests();
    let wire = state(&["waitingOnApproval"], true);
    replay.streams(wire.clone());
    let id = named(&requests).unwrap().remove(0);
    replay.owner_confirms();
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0));
    // The reply worker has taken the ticket, as when the answer was sent.
    replay
        .observer
        .shared
        .state
        .lock()
        .unwrap()
        .pending
        .get_mut(&id)
        .unwrap()
        .ticket = None;
    replay.streams(wire);
    let state = replay.observer.shared.state.lock().unwrap();
    assert!(state.pending[&id].ticket.is_none(), "answered twice");
    drop(state);
    assert_eq!(requests.auto_approved(crate::hooks::now_ms()).len(), 1);
    assert_eq!(replay.look(), ("waiting".to_owned(), 0, 0));
    replay.observer.stop();
}

// Agent HUD Fork (ticket 74): two requests one after the other in one wait.
#[path = "asks_tests.rs"]
mod asks;
