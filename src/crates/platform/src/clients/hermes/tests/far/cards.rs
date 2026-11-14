//! Cards from a far gateway: what it asks shows, an answer given on the
//! card reaches it and the turn goes on, and what settles there goes.
use super::{
    super::{fake::Fake, fake::PROFILE, lab as local_lab, quick, until as soon, Observer},
    far_lab, until, FarLab, LABEL,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// The last call of `method` the gateway got, once there is one.
fn sent(gateway: &Fake, method: &str) -> Option<Value> {
    let calls = gateway.world().calls(method);
    calls.last().map(|(_, frame)| frame["params"].clone())
}

fn following(instance: &str) -> Option<(FarLab, Fake)> {
    let mut far = far_lab()?;
    let gateway = far.gateway(instance);
    far.follow();
    far.followed();
    Some((far, gateway))
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_question_shows_and_is_answered_by_option_and_by_typed_words() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    let key = far.followed();
    // No hint is said: the poll alone finds it.
    let session = gateway.play("question-answered-here", Some("clarify.request"));
    let card = far.card();
    let (key, id) = (key.as_str(), card["id"].as_str().unwrap().to_owned());
    assert_eq!(card["kind"], "question");
    assert_eq!(card["machine"], LABEL);
    assert_eq!(
        card["sessionId"],
        format!("hermes:{key}/{PROFILE}:{session}")
    );
    assert!(id.starts_with(&format!("hermes:{key}/default:{PROFILE}:{session}:")));
    let answer_key = card["questions"][0]["answerKey"].as_str().unwrap();

    // By option: Hermes hears it, settles the request, and the turn goes on.
    let picked = json!({"kind": "answer", "answers": {answer_key: {"selected": ["green"]}}});
    far.answer(&id, &picked);
    until("Hermes hears the answer", || {
        sent(&gateway, "clarify.respond").is_some()
    });
    assert_eq!(
        sent(&gateway, "clarify.respond").unwrap()["answer"],
        "green"
    );
    assert_eq!(far.delivery(&id)["state"], "confirmed");
    until("the card goes", || far.cards().is_empty());
    assert!(gateway.world().stream(&session).prompts.is_empty());
    assert!(gateway.world().stream(&session).running, "the turn goes on");

    // Another, with no options, answered by typed words.
    let session = gateway.play("question-open-ended", Some("clarify.request"));
    let card = far.card();
    let id = card["id"].as_str().unwrap().to_owned();
    assert_eq!(card["questions"][0]["options"], json!([]));
    let answer_key = card["questions"][0]["answerKey"].as_str().unwrap();
    let typed = json!({"kind": "answer", "answers":
        {answer_key: {"selected": [], "custom": "Synthetic Widget"}}});
    far.answer(&id, &typed);
    until("Hermes hears the words", || {
        let said = sent(&gateway, "clarify.respond");
        said.is_some_and(|said| said["answer"] == "Synthetic Widget")
    });
    assert_eq!(far.delivery(&id)["state"], "confirmed");
    until("the card goes", || far.cards().is_empty());
    assert!(gateway.world().stream(&session).prompts.is_empty());
    println!("the log of one far question answered:\n{}", far.log());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_approval_is_answered_with_each_choice_it_offers() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    // Deny, the two lasting choices as the card offers them, allow once:
    // one approval for each, all on the one link.
    let recordings = [
        ("approval-answered-deny", "deny"),
        ("approval-answered-session", "session"),
        ("approval-answered-always", "always"),
        ("approval-answered-once", "once"),
    ];
    for (recording, word) in recordings {
        gateway.play(recording, Some("approval.request"));
        let card = far.card();
        assert_eq!(card["machine"], LABEL, "{card}");
        let lasting = card["suggestions"].as_array().unwrap().clone();
        assert_eq!(lasting.len(), 2, "{card}");
        let decision = match word {
            "deny" => json!({"kind": "deny"}),
            "session" => json!({"kind": "always", "suggestion": lasting[0]}),
            "always" => json!({"kind": "always", "suggestion": lasting[1]}),
            _ => json!({"kind": "allow"}),
        };
        let id = card["id"].as_str().unwrap().to_owned();
        let before = gateway.world().calls("approval.respond").len();
        far.answer(&id, &decision);
        until("Hermes hears the choice", || {
            gateway.world().calls("approval.respond").len() > before
        });
        let said = sent(&gateway, "approval.respond").unwrap();
        assert_eq!(said["choice"], json!(word), "{decision}");
        assert_eq!(far.delivery(&id)["state"], "confirmed");
        until("the card goes", || far.cards().is_empty());
    }
    assert_eq!(gateway.world().calls("approval.respond").len(), 4);
    assert_eq!(far.logins(), 2, "the lab's own check, and the one link");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_request_settled_on_the_far_side_removes_its_card() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    gateway.play("question-answered-elsewhere", Some("clarify.request"));
    let card = far.card();
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the card goes", || far.cards().is_empty());
    assert!(far.log().contains("why=Settled"), "{}", far.log());
    // Nothing was answered here, so nothing was sent and nothing noted.
    assert!(gateway.world().calls("clarify.respond").is_empty());
    let id = card["id"].as_str().unwrap();
    let rows = far.requests.deliveries(crate::hooks::now_ms());
    assert!(rows.iter().all(|row| row["id"] != id), "{rows:?}");
}

/// The same profile and the same session id on this machine and on a far
/// one are two sessions, each with its own card, and an answer to one
/// goes to its own gateway.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_and_a_local_session_of_one_name_are_two_sessions_with_two_cards() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let there = far.gateway("instance-far");
    // This machine's own gateway, told to the same core.
    let mut here_lab = local_lab();
    here_lab.requests = far.requests.clone();
    let here = Fake::start(&here_lab.home, "instance-here");
    let wake = Arc::new(|| {});
    let (route, requests) = (here_lab.route.clone(), far.requests.clone());
    let local = Observer::start_on(route, &here_lab.data, requests, wake, quick()).unwrap();
    far.follow();
    let key = far.followed();

    let session = here.play("question-answered-here", Some("clarify.request"));
    assert_eq!(
        there.play("question-answered-here", Some("clarify.request")),
        session
    );
    until("both cards show", || far.cards().len() == 2);
    let cards = far.cards();
    let of = |machine: Option<&str>| {
        let its = |card: &&Value| card["machine"].as_str() == machine;
        cards.iter().find(its).expect("the card").clone()
    };
    let (near, distant) = (of(None), of(Some(LABEL)));
    assert_eq!(near["sessionId"], format!("hermes:{PROFILE}:{session}"));
    let far_session = format!("hermes:{}/{PROFILE}:{session}", key.as_str());
    assert_eq!(distant["sessionId"], far_session);
    assert_ne!(near["id"], distant["id"]);
    let told = far.requests.native_waiting_sessions();
    let mut waiting: Vec<String> = told["hermes"].iter().cloned().collect();
    waiting.sort();
    let mut both = vec![format!("hermes:{PROFILE}:{session}"), far_session];
    both.sort();
    assert_eq!(waiting, both, "two sessions wait");

    // The far card's answer goes to the far gateway alone.
    let answer_key = distant["questions"][0]["answerKey"].as_str().unwrap();
    let picked = json!({"kind": "answer", "answers": {answer_key: {"selected": ["green"]}}});
    far.answer(distant["id"].as_str().unwrap(), &picked);
    until("the far gateway hears it", || {
        sent(&there, "clarify.respond").is_some()
    });
    until("the far card goes", || far.cards().len() == 1);
    assert!(here.world().calls("clarify.respond").is_empty());
    assert_eq!(far.cards()[0]["id"], near["id"]);
    soon("the local card still shows", || far.cards().len() == 1);
    drop(local);
}
