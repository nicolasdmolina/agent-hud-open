//! A Hermes question on its way through Core, as the connection (ticket 13)
//! carries it: the gateway model's card is registered as a native request,
//! the HUD shows it under the profile's label and sounds `needs-you` like
//! any other request, and the decision the card sends comes back on the
//! ticket and becomes the exact frame Hermes gets. The requests are the
//! recorded ones in the agents crate's gateway fixtures.
use super::*;
use agent_hud_agents::hermes::gateway::{
    frame, Ask, Frame, Gateway, GatewayId, Pending, Profile, Reply, Scope,
};

fn recorded(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../agents/tests/fixtures/hermes/gateway")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The first question (or set of questions) the recording `name` asks, as
/// the gateway model holds it, in the throwaway profile it was recorded in.
pub(super) fn asked(name: &str) -> Pending {
    let gateway = Gateway::new(GatewayId::new("local"));
    let profile = Profile::named("hud-spike");
    let doc = recorded(name);
    for entry in doc["clients"]["hud"].as_array().unwrap() {
        if entry["dir"] != "in" {
            continue;
        }
        let found = match frame(&entry["frame"]) {
            Frame::Event(event) if event.kind == "clarify.request" => Ask::prompt(event.payload)
                .map(|(ask, wait_ms)| (event.session.to_owned(), ask, wait_ms)),
            Frame::Request { id, method, params } => Ask::batch(id, method, params)
                .map(|ask| (params["session_id"].as_str().unwrap().to_owned(), ask, None)),
            _ => None,
        };
        if let Some((session, ask, wait_ms)) = found {
            let session = gateway.session(&profile, &session);
            return Pending {
                session,
                ask,
                wait_ms,
            };
        }
    }
    panic!("{name} asks nothing");
}

/// A Core whose Hermes home has the profile `hud-spike` with the sessions
/// `ids`, read once, with sound on.
pub(super) fn hermes_core(temp: &Path, ids: &[&str]) -> AgentHudCore {
    let home = temp.join(".hermes");
    let folder = home.join("profiles/hud-spike");
    fs::create_dir_all(&folder).unwrap(); // plain directory
    let db = rusqlite::Connection::open(folder.join("state.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, started_at REAL NOT NULL, input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0, title TEXT, profile_name TEXT);",
    )
    .unwrap();
    let now = Utc::now().timestamp();
    for id in ids {
        db.execute(
            "INSERT INTO sessions VALUES (?1, 'tui', 'loopback', ?2, 10, 3, 'Pick a colour', 'hud-spike')",
            rusqlite::params![id, now - 60],
        )
        .unwrap();
    }
    let mut core = AgentHudCore::new(vec![], temp.join("app"));
    core.provider_roots = vec![(home, "hermes")];
    core.apply_setting("sound", &json!(true)).unwrap();
    core.request("refresh", json!({})).unwrap();
    for _ in 0..10 {
        if core.reads.pending.is_empty() {
            break;
        }
        core.request("snapshot", json!({})).unwrap();
    }
    core
}

/// What the connection does with `Effect::Open`: the card, and the session
/// marked as waiting.
pub(super) fn open(core: &AgentHudCore, pending: &Pending) -> requests::Ticket {
    let now = Utc::now().timestamp_millis();
    let ticket = core
        .requests()
        .register_native(pending.id(), pending.view(now), now)
        .expect("a new card");
    core.requests()
        .set_native_waiting("hermes", &[pending.session.hud_id()], now);
    ticket
}

/// The frame a decision given on the HUD sends: through `answer_request`,
/// onto the ticket, and through the gateway model's encoding.
fn answered(core: &mut AgentHudCore, pending: &Pending, decision: Value) -> Option<Value> {
    let ticket = open(core, pending);
    core.request(
        "answer_request",
        json!({"id": pending.id(), "decision": decision}),
    )
    .unwrap_or_else(|error| panic!("{error}: {decision} on {}", pending.view(0)));
    let given = ticket.response.recv().unwrap().expect("a decision");
    assert_eq!(given, decision);
    match pending.reply(&given, Scope::Host).unwrap()? {
        Reply::Call(call) => Some(call.frame(7)),
        Reply::Result(whole) => Some(whole),
    }
}

pub(super) fn needs_you(snapshot: &Value) -> Vec<Value> {
    snapshot["soundAlerts"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|alert| alert["kind"] == "needs-you")
        .map(|alert| alert["sessionId"].clone())
        .collect()
}

pub(super) fn alert_ids(snapshot: &Value) -> Vec<Value> {
    snapshot["soundAlerts"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|alert| alert["id"].clone())
        .collect()
}

#[test]
fn a_hermes_question_shows_under_its_profile_and_sounds_once() {
    let temp = tempfile::tempdir().unwrap();
    let question = asked("question-answered-here");
    let mut core = hermes_core(temp.path(), &[&question.session.id]);
    // What waits at the first look is never told.
    core.request("snapshot", json!({})).unwrap();

    let _ticket = open(&core, &question);
    let snapshot = core.request("snapshot", json!({})).unwrap();
    let card = snapshot["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == question.id().as_str())
        .expect("the card")
        .clone();
    assert_eq!(card["vendor"], "Hermes Agent · hud-spike");
    assert_eq!(card["kind"], "question");
    assert_eq!(
        card["questions"][0]["question"],
        "Which color should the widget be?"
    );
    let session = question.session.hud_id();
    assert_eq!(card["sessionId"], session.as_str());
    assert_eq!(needs_you(&snapshot), [json!(session)]);
    // The same card at the next look makes no second sound: the alert
    // keeps its id, which the host plays once.
    let again = core.request("snapshot", json!({})).unwrap();
    assert_eq!(alert_ids(&again), alert_ids(&snapshot));

    // Answered in Hermes's own window: the connection settles it, the card
    // goes, and nothing sounds.
    core.requests().settle_native(&question.id());
    core.requests().set_native_waiting("hermes", &[], 0);
    let after = core.request("snapshot", json!({})).unwrap();
    assert!(!after["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|card| card["id"] == question.id().as_str()));
    assert_eq!(alert_ids(&after), alert_ids(&snapshot));
}

#[test]
fn a_decision_on_the_card_becomes_the_exact_frame_hermes_gets() {
    let temp = tempfile::tempdir().unwrap();
    let picked = asked("question-answered-here");
    let typed = asked("question-open-ended");
    let set = asked("question-batch-answered-here");
    let mut core = hermes_core(
        temp.path(),
        &[&picked.session.id, &typed.session.id, &set.session.id],
    );
    let call = |pending: &Pending, prompt: &str, answer: &str| {
        json!({"jsonrpc": "2.0", "id": 7, "method": "clarify.respond", "params": {
            "session_id": pending.session.id, "prompt_id": prompt,
            "execution_generation": 1, "answer": answer, "profile": "hud-spike",
        }})
    };

    // An option chosen.
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    assert_eq!(
        answered(&mut core, &picked, decision),
        Some(call(&picked, "715a2db63d", "green"))
    );

    // Words typed where no options are offered.
    let decision = json!({"kind": "answer", "answers": {"cd068655b1": {"selected": [], "custom": "Synthetic Widget"}}});
    assert_eq!(
        answered(&mut core, &typed, decision),
        Some(call(&typed, "cd068655b1", "Synthetic Widget"))
    );

    // A lone question has no Skip: Core refuses a decision with no answer
    // and the card stays. Sent as the empty answer, it would end Hermes's
    // call as timed out, with any question still to come in it.
    let mut lone = picked.clone();
    if let Ask::Question(question) = &mut lone.ask {
        question.prompt.id = "1a8bbfc16d".into();
    }
    let _ticket = open(&core, &lone);
    assert_eq!(
        core.request(
            "answer_request",
            json!({"id": lone.id(), "decision": {"kind": "answer", "answers": {}}}),
        ),
        Err("Every question needs an answer".into())
    );
    let snapshot = core.request("snapshot", json!({})).unwrap();
    assert!(snapshot["approvals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|card| card["id"] == lone.id().as_str() && card["canSkip"] == false));

    // A set of questions, one of them already answered by another viewer:
    // the card shows that answer, and only the question answered here is
    // written; the other keeps what the other viewer gave.
    let Ask::Batch(batch) = &set.ask else {
        panic!("a set of questions");
    };
    let mut locked = set.clone();
    if let Ask::Batch(read) = &mut locked.ask {
        read.questions[0].locked = Some(agent_hud_agents::hermes::gateway::Locked::Answered(
            "blue".into(),
        ));
    }
    let view = locked.view(0);
    assert_eq!(
        view["questions"][0]["lock"],
        json!({"state": "answered", "answer": "blue"})
    );
    assert_eq!(view["questions"][1]["lock"], Value::Null);
    let decision = json!({"kind": "answer", "answers": {"q1": {"selected": ["small (Recommended)", "large"]}}});
    assert_eq!(
        answered(&mut core, &locked, decision),
        Some(json!({"jsonrpc": "2.0", "id": batch.request, "result": {
            "answers": {"q1": r#"["small (Recommended)","large"]"#},
        }}))
    );

    // Skip on every question settles the set: each question skipped here
    // goes as `null`, and one locked by another viewer keeps its answer. A
    // set answered once is never shown again, so each case is a set of its
    // own.
    let again = |pending: &Pending, request: &str| {
        let mut pending = pending.clone();
        if let Ask::Batch(read) = &mut pending.ask {
            read.request = request.into();
        }
        pending
    };
    let skip_all = json!({"kind": "answer", "answers": {}});
    let all = again(&set, "srq-all-skipped");
    assert_eq!(
        answered(&mut core, &all, skip_all.clone()),
        Some(
            json!({"jsonrpc": "2.0", "id": "srq-all-skipped", "result": {
                "answers": {"q0": null, "q1": null},
            }})
        )
    );
    let half = again(&locked, "srq-locked-and-skipped");
    assert_eq!(
        answered(&mut core, &half, skip_all),
        Some(
            json!({"jsonrpc": "2.0", "id": "srq-locked-and-skipped", "result": {
                "answers": {"q1": null},
            }})
        )
    );
}
