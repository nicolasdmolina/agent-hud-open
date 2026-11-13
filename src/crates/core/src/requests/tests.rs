//! A native request whose card offers lasting choices of its own, and a
//! native request whose wait runs out.
use super::Requests;
use serde_json::{json, Value};

/// A card that offers allowing for the session and always, each labelled,
/// as a client adapter that knows such choices builds it.
fn scoped(lasting: &[&str]) -> Value {
    let suggestions: Vec<Value> = lasting
        .iter()
        .map(|scope| json!({"type": "scope", "scope": scope, "label": scope}))
        .collect();
    json!({"kind": "permission", "source": "client", "sessionId": "s", "title": "t",
        "allowsAlways": !suggestions.is_empty(), "suggestions": suggestions})
}

fn always(scope: &str) -> Value {
    json!({"kind": "always", "suggestion": {"type": "scope", "scope": scope, "label": scope}})
}

#[test]
fn each_lasting_choice_reaches_the_adapter_as_it_was_chosen() {
    let requests = Requests::default();
    for (n, decision) in [
        json!({"kind": "allow"}),
        always("session"),
        always("always"),
        json!({"kind": "deny"}),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("native:{n}");
        let ticket = requests
            .register_native(id.clone(), scoped(&["session", "always"]), 0)
            .unwrap();
        assert!(requests.checked(&id, &decision, 1).is_ok(), "{decision}");
        requests.answer(&id, &decision, 1).unwrap();
        assert_eq!(ticket.response.recv().unwrap(), Some(decision));
    }
}

#[test]
fn a_card_with_no_lasting_choice_takes_only_allow_and_deny() {
    let requests = Requests::default();
    requests.register_native("native:1".into(), scoped(&[]), 0);
    for decision in [
        always("session"),
        always("always"),
        json!({"kind": "always"}),
    ] {
        assert!(
            requests.checked("native:1", &decision, 1).is_err(),
            "{decision}"
        );
        assert!(
            requests.answer("native:1", &decision, 1).is_err(),
            "{decision}"
        );
    }
    for decision in [json!({"kind": "allow"}), json!({"kind": "deny"})] {
        assert!(
            requests.checked("native:1", &decision, 1).is_ok(),
            "{decision}"
        );
    }
}

/// When the HUD's wait runs out, the adapter hears no decision: the card
/// goes and the request is left to the client's own windows. The same
/// request read again does not come back until the client settles it.
#[test]
fn a_wait_that_runs_out_answers_nothing() {
    let requests = Requests::default();
    requests.set_hold_ms(1_000);
    let ticket = requests
        .register_native("native:1".into(), scoped(&["session", "always"]), 0)
        .unwrap();
    requests.expire(999);
    assert_eq!(requests.views(999).len(), 1);
    requests.expire(1_000);
    assert_eq!(ticket.response.recv().unwrap(), None);
    assert!(requests.views(1_000).is_empty());
    assert!(requests
        .answer("native:1", &json!({"kind": "deny"}), 1_000)
        .is_err());
    assert!(requests
        .register_native("native:1".into(), scoped(&["session", "always"]), 1_001)
        .is_none());
    requests.settle_native("native:1");
    assert!(requests
        .register_native("native:1".into(), scoped(&["session", "always"]), 1_002)
        .is_some());
}
