//! An approval card's buttons through the whole connection (ticket 15):
//! each decision the card sends, with the suggestion it carries, reaches
//! the gateway as Hermes's own answer; a card Hermes limits takes no lasting
//! choice; and when the HUD's wait runs out nothing reaches the gateway,
//! the request is left to Hermes, and the card does not come back.
use super::{fake::Fake, lab, until};
use serde_json::{json, Value};
use std::time::Duration;

const RECORDING: &str = "approval-answered-once";

/// The choice Hermes was sent, once one was.
fn sent(gateway: &Fake) -> Option<Value> {
    let calls = gateway.world().calls("approval.respond");
    calls
        .last()
        .map(|(_, frame)| frame["params"]["choice"].clone())
}

#[test]
fn each_button_on_the_card_is_the_answer_hermes_gets() {
    // Deny, the two lasting choices as the card offers them, allow once.
    let buttons = |card: &Value| {
        let lasting = card["suggestions"].as_array().unwrap().clone();
        assert_eq!(lasting.len(), 2, "{card}");
        vec![
            (json!({"kind": "deny"}), "deny"),
            (
                json!({"kind": "always", "suggestion": lasting[0]}),
                "session",
            ),
            (
                json!({"kind": "always", "suggestion": lasting[1]}),
                "always",
            ),
            (json!({"kind": "allow"}), "once"),
        ]
    };
    for index in 0..4 {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        lab.hears(&gateway, RECORDING, "approval.request");
        let card = lab.card();
        let (decision, word) = buttons(&card).swap_remove(index);
        let id = card["id"].as_str().unwrap();
        lab.requests
            .answer(id, &decision, crate::hooks::now_ms())
            .unwrap();
        until("the answer reaches the gateway", || {
            sent(&gateway).is_some()
        });
        assert_eq!(sent(&gateway), Some(json!(word)), "{decision}");
        assert_eq!(gateway.world().calls("approval.respond").len(), 1);
    }
}

/// Upstream lets a denial carry the user's words (`message`) to hook
/// clients. Hermes's `approval.respond` takes only a choice, so a denial
/// with words still reaches it as exactly one bare `deny` (FORK.md, ticket 17).
#[test]
fn a_denial_with_words_reaches_hermes_as_a_bare_deny() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, RECORDING, "approval.request");
    let card = lab.card();
    let id = card["id"].as_str().unwrap();
    let decision = json!({"kind": "deny", "message": "Use the staging folder instead."});
    lab.requests
        .answer(id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the answer reaches the gateway", || {
        sent(&gateway).is_some()
    });
    let calls = gateway.world().calls("approval.respond");
    assert_eq!(calls.len(), 1);
    let params = &calls[0].1["params"];
    assert_eq!(params["choice"], json!("deny"));
    assert!(!params.to_string().contains("staging"), "{params}");
}

#[test]
fn a_limited_card_takes_no_lasting_choice() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "approval-limited-choice", "approval.request");
    let card = lab.card();
    assert_eq!(card["suggestions"], json!([]));
    assert_eq!(card["allowsAlways"], false);
    let id = card["id"].as_str().unwrap();
    let session = json!({"kind": "always", "suggestion": {"type": "scope", "scope": "session"}});
    assert!(lab
        .requests
        .answer(id, &session, crate::hooks::now_ms())
        .is_err());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(sent(&gateway), None);
    assert_eq!(
        lab.cards().len(),
        1,
        "the card still waits for an answer it takes"
    );
}

#[test]
fn a_wait_that_runs_out_in_the_hud_sends_hermes_nothing() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, RECORDING, "approval.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    // The HUD's wait for this card is over.
    lab.requests.set_hold_ms(1);
    lab.requests.expire(crate::hooks::now_ms() + 10);
    assert!(lab.cards().is_empty());
    // Polls go on and still find the request waiting in Hermes: it is
    // never answered and its card does not come back.
    let shared = lab.observer.as_ref().unwrap().local();
    for _ in 0..5 {
        shared.want_poll();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(sent(&gateway), None);
    assert!(lab.cards().is_empty());
    assert!(!lab.log().contains("answer "), "{}", lab.log());
    assert!(lab
        .requests
        .answer(&id, &json!({"kind": "deny"}), 0)
        .is_err());
}
