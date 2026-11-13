//! Cards: what Hermes asks shows, what it settles goes, and an answer
//! given on a card reaches it on a socket of its own.
use super::{fake::fixture, fake::Fake, fake::PROFILE, lab, until};
use serde_json::{json, Value};

/// The frame the recorded client sent to `method`, as the connection must
/// send it: the same parameters, and the profile a host's socket names.
fn recorded_call(recording: &str, method: &str) -> Value {
    let recorded = fixture(recording);
    let frames = recorded["clients"]["hud"].as_array().unwrap();
    let sent = |entry: &&Value| entry["dir"] == "out" && entry["frame"]["method"] == method;
    let mut params = frames.iter().find(sent).unwrap()["frame"]["params"].clone();
    params["profile"] = json!(PROFILE);
    params
}

#[test]
fn what_hermes_asks_shows_and_goes_when_it_is_settled_elsewhere() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    let session = lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let card = lab.card();
    assert_eq!(
        card["id"],
        format!("hermes:local:{PROFILE}:{session}:e1bc9d4fa6")
    );
    assert_eq!(card["kind"], "question");
    assert_eq!(card["sessionId"], format!("hermes:{PROFILE}:{session}"));
    assert_eq!(
        card["questions"][0]["question"],
        "Which color should the widget be?"
    );
    let waits = [format!("hermes:{PROFILE}:{session}")];
    until("core is told the session waits", || lab.waiting() == waits);

    // The other client answers: Hermes says it settled, and the card goes.
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the card goes", || lab.cards().is_empty());
    until("the session stops waiting", || lab.waiting().is_empty());
    assert!(lab.log().contains("why=Settled"), "{}", lab.log());
    // Reading what waits never subscribed to the session.
    assert!(gateway.world().calls("session.resume").is_empty());
    assert_eq!(gateway.world().sockets, 1);
}

#[test]
fn an_answer_given_on_a_card_goes_to_hermes_on_a_socket_of_its_own() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();

    until("Hermes hears the answer", || {
        !gateway.world().calls("session.detach").is_empty()
    });
    let world = gateway.world();
    let (socket, sent) = world.calls("clarify.respond").remove(0);
    assert_eq!(
        sent["params"],
        recorded_call("question-answered-here", "clarify.respond")
    );
    // Not the socket the polls run on, and on it: subscribe, answer, detach.
    assert_ne!(socket, 1);
    let on_it: Vec<&str> = world
        .received
        .iter()
        .filter(|(number, _)| *number == socket)
        .filter_map(|(_, frame)| frame["method"].as_str())
        .collect();
    assert_eq!(
        on_it,
        ["session.resume", "clarify.respond", "session.detach"]
    );
    // Detaching never ends the session itself.
    let (_, parting) = world.calls("session.detach").remove(0);
    assert!(parting["params"].get("end_reason").is_none());
    drop(world);

    until("the card goes", || lab.cards().is_empty());
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    // Closed by Hermes's reply, or by a poll that saw the request settled
    // a moment before the reply was handled: the receipt is the same.
    let log = lab.log();
    assert!(
        log.contains("why=Answered") || log.contains("why=Settled"),
        "{log}"
    );
}

#[test]
fn an_approval_takes_allow_once_as_hermes_recorded_it() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "approval-answered-once", "approval.request");
    let card = lab.card();
    assert_eq!(card["kind"], "permission");
    let id = card["id"].as_str().unwrap().to_owned();
    let allow = json!({"kind": "allow"});
    lab.requests
        .answer(&id, &allow, crate::hooks::now_ms())
        .unwrap();
    until("Hermes hears the answer", || {
        !gateway.world().calls("approval.respond").is_empty()
    });
    let (_, sent) = gateway.world().calls("approval.respond").remove(0);
    assert_eq!(
        sent["params"],
        recorded_call("approval-answered-once", "approval.respond")
    );
    until("the card goes", || lab.cards().is_empty());
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
}

#[test]
fn a_set_of_questions_is_answered_with_a_result_frame_and_closes_at_the_next_poll() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-batch-answered-here", "clarify");
    let card = lab.card();
    assert_eq!(card["questions"].as_array().unwrap().len(), 2);
    let id = card["id"].as_str().unwrap().to_owned();
    // Hermes marks the first choice of each question as recommended.
    let decision = json!({"kind": "answer", "answers": {
        "q0": {"selected": ["green"]},
        "q1": {"selected": ["small (Recommended)", "large"]},
    }});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    let result = |(_, frame): &(usize, Value)| frame["id"] == "srq-be9ca8d8cb91";
    until("Hermes hears the answers", || {
        gateway.world().received.iter().any(result)
    });
    let world = gateway.world();
    let (_, sent) = world.received.iter().find(|entry| result(entry)).unwrap();
    assert_eq!(
        sent["result"],
        json!({"answers": {"q0": "green", "q1": "[\"small (Recommended)\",\"large\"]"}})
    );
    assert!(world.calls("clarify.respond").is_empty());
    drop(world);
    // Nothing replies to it: the poll that no longer lists the set closes it.
    until("the card goes", || lab.cards().is_empty());
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
}

#[test]
fn an_answer_that_came_too_late_closes_the_card_and_says_so() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    gateway.world().answers_with = Some("already_resolved");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let decision = json!({"kind": "answer", "answers": {"e1bc9d4fa6": {"selected": ["red"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    let receipt = lab.delivery(&id);
    assert_eq!(receipt["state"], "failed");
    assert!(receipt["reason"]
        .as_str()
        .is_some_and(|why| !why.is_empty()));
    assert!(lab.cards().is_empty());
    // Closed by Hermes's reply, or by a poll a moment before it.
    let log = lab.log();
    assert!(
        log.contains("why=AlreadyResolved") || log.contains("why=Settled"),
        "{log}"
    );
}

#[test]
fn an_answer_hermes_does_not_take_brings_the_card_back() {
    for (reply, state) in [("invalid_params", "failed"), ("silence", "uncertain")] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        gateway.world().answers_with = Some(reply);
        lab.follow();
        lab.hears(&gateway, "question-answered-here", "clarify.request");
        let id = lab.card()["id"].as_str().unwrap().to_owned();
        let decision =
            json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
        lab.requests
            .answer(&id, &decision, crate::hooks::now_ms())
            .unwrap();
        // Core put the card away when it was answered. The request still
        // waits in Hermes, so its card shows again, under the same id, with
        // a receipt that says what happened, and can be answered again.
        assert_eq!(lab.delivery(&id)["state"], state, "{reply}");
        assert_eq!(lab.card()["id"], id.as_str(), "{reply}");
        gateway.world().answers_with = None;
        lab.requests
            .answer(&id, &decision, crate::hooks::now_ms())
            .unwrap();
        until("the second answer lands", || {
            lab.delivery(&id)["state"] == "confirmed"
        });
        until("the card goes", || lab.cards().is_empty());
    }
}

#[test]
fn an_answer_that_cannot_be_sent_brings_the_card_back() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    // The gateway is gone, and the card still shows: the request may wait.
    gateway.kill();
    until("the loss is seen", || {
        lab.log().contains("connection failed")
    });
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    // Core put the card away on the answer. Nothing could be sent, so it
    // comes back under the same id, and the receipt says why.
    let receipt = lab.delivery(&id);
    assert_eq!(receipt["state"], "failed");
    assert!(receipt["reason"]
        .as_str()
        .is_some_and(|why| !why.is_empty()));
    assert_eq!(lab.card()["id"], id.as_str());
}

#[test]
fn a_decision_with_nothing_to_send_is_done_and_not_a_failed_answer() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    // Leaving a card alone: core lets go of it and nothing goes to Hermes.
    let leave = json!({"kind": "leave"});
    lab.requests
        .answer(&id, &leave, crate::hooks::now_ms())
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(lab.cards().is_empty());
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty() && world.calls("session.resume").is_empty());
    drop(world);
    // The request still waits in Hermes, and core is still told so.
    assert_eq!(lab.waiting().len(), 1);
    // When Hermes settles it, the card is over for good.
    gateway.play("question-answered-here", Some("message.complete"));
    until("the session stops waiting", || lab.waiting().is_empty());
    assert!(lab.log().contains("close id="), "{}", lab.log());
}

#[test]
fn stopping_withdraws_every_card() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    lab.card();
    // Turning hooks off drops the observer.
    lab.observer.take();
    assert!(lab.cards().is_empty());
    assert!(lab.waiting().is_empty());
    assert!(lab.log().ends_with("stopped\n"));
}
