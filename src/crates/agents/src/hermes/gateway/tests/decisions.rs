//! Every decision a card can send, as the exact frame Hermes gets, and the
//! card each kind of request shows. The requests are the recorded ones.
use super::super::*;
use super::{fixture, local, spike};
use serde_json::{json, Value};

/// The first request of `kind` in a recording, as the model holds it.
fn asked(name: &str, kind: &str) -> Pending {
    let doc = fixture(name);
    let mut gateway = local();
    for entry in doc["clients"]["hud"].as_array().unwrap() {
        let sent = &entry["frame"];
        let is = sent["params"]["type"] == kind || (kind == "clarify" && sent["method"] == kind);
        if entry["dir"] == "in" && is {
            let effects = gateway.apply(
                Input::Frame {
                    run: gateway.run(),
                    profile: &spike(),
                    frame: sent,
                },
                1_000,
            );
            if let Some(Effect::Open(pending)) = effects.into_iter().next() {
                return pending;
            }
        }
    }
    panic!("{name} asks no {kind}");
}

fn pick(key: &str, selected: &[&str], custom: Option<&str>) -> Value {
    json!({"kind": "answer", "answers": {key: {"selected": selected, "custom": custom}}})
}

fn scope(word: &str) -> Value {
    json!({"kind": "always", "suggestion": {"type": "scope", "scope": word}})
}

/// The frame a decision sends with id 7, or why it sends none.
fn sent(pending: &Pending, decision: &Value, via: Scope) -> Result<Option<Value>, String> {
    Ok(match pending.reply(decision, via)? {
        Some(Reply::Call(call)) => Some(call.frame(7)),
        Some(Reply::Result(whole)) => Some(whole),
        None => None,
    })
}

const ONLY_ANSWERED: &str = "Hermes's questions can only be answered, not approved.";
const ONLY_APPROVED: &str = "Hermes's approvals can only be approved or denied.";
const NOT_OFFERED: &str = "Hermes did not offer that choice for this request.";
const EMPTY: &str = "Choose an option or type an answer.";

#[test]
fn a_question_takes_words_and_nothing_else() {
    let question = asked("question-answered-here", "clarify.request");
    let session = "local-f200386683510182cd45c8e38361a3bfecfa23934bacf51c394ec5b828f7ee9b";
    let key = "715a2db63d";
    let frame = |answer: &str| {
        Ok(Some(
            json!({"jsonrpc": "2.0", "id": 7, "method": "clarify.respond", "params": {
                "session_id": session, "prompt_id": key, "execution_generation": 1, "answer": answer,
            }}),
        ))
    };
    let table: &[(Value, Result<Option<Value>, String>)] = &[
        (pick(key, &["green"], None), frame("green")),
        // The gateway's own mark on its first choice goes back as shown.
        (
            pick(key, &["blue (Recommended)"], None),
            frame("blue (Recommended)"),
        ),
        (pick(key, &[], Some("teal")), frame("teal")),
        (pick(key, &["green"], Some(" teal ")), frame("teal")),
        // An empty answer would be taken as no answer: it is never sent.
        (pick(key, &[], None), Err(EMPTY.into())),
        (pick(key, &[""], Some("  ")), Err(EMPTY.into())),
        (pick("another-key", &["green"], None), Err(EMPTY.into())),
        (json!({"kind": "allow"}), Err(ONLY_ANSWERED.into())),
        (json!({"kind": "deny"}), Err(ONLY_ANSWERED.into())),
        (json!({"kind": "always"}), Err(ONLY_ANSWERED.into())),
        (scope("session"), Err(ONLY_ANSWERED.into())),
        (json!({"kind": "leave"}), Ok(None)),
    ];
    for (decision, expected) in table {
        assert_eq!(
            sent(&question, decision, Scope::Profile),
            *expected,
            "{decision}"
        );
    }
    // On a socket for the whole host, the call names the session's profile.
    let hosted = sent(&question, &pick(key, &["green"], None), Scope::Host)
        .unwrap()
        .unwrap();
    assert_eq!(hosted["params"]["profile"], "hud-spike");
    assert_eq!(hosted["params"].as_object().unwrap().len(), 5);

    // A question with no choices is answered in the person's own words.
    let open = asked("question-open-ended", "clarify.request");
    let typed = sent(
        &open,
        &pick("cd068655b1", &[], Some("Synthetic Widget")),
        Scope::Profile,
    );
    assert_eq!(
        typed.unwrap().unwrap()["params"]["answer"],
        "Synthetic Widget"
    );
}

#[test]
fn several_picks_go_as_a_list_in_the_answer() {
    let doc = fixture("session-ids-and-hooks");
    let mut gateway = local();
    let mut several = None;
    for entry in doc["clients"]["hud"].as_array().unwrap() {
        let effects = gateway.apply(
            Input::Frame {
                run: gateway.run(),
                profile: &spike(),
                frame: &entry["frame"],
            },
            0,
        );
        for effect in effects {
            if let Effect::Open(pending) = effect {
                if matches!(&pending.ask, Ask::Question(q) if q.multi_select) {
                    several = Some(pending);
                }
            }
        }
    }
    let several = several.unwrap();
    let key = "0facc34e08";
    let answer = |decision: Value| {
        let frame = sent(&several, &decision, Scope::Profile).unwrap().unwrap();
        assert_eq!(frame["params"]["execution_generation"], 2);
        frame["params"]["answer"].as_str().unwrap().to_owned()
    };
    assert_eq!(
        answer(pick(key, &["small", "large"], None)),
        r#"["small","large"]"#
    );
    assert_eq!(answer(pick(key, &["small"], None)), r#"["small"]"#);
    assert_eq!(
        answer(pick(key, &["small"], Some("tiny"))),
        r#"["small","tiny"]"#
    );
    // Words typed alone go as they are; Hermes reads them as one answer.
    assert_eq!(answer(pick(key, &[], Some("small, large"))), "small, large");
    // What the recording sent, and Hermes took as two picks, is the same list.
    let recorded = doc["notes"]["multi_select_answer_sent"].as_str().unwrap();
    let list = |text: &str| serde_json::from_str::<Vec<String>>(text).unwrap();
    assert_eq!(
        list(&answer(pick(key, &["small", "large"], None))),
        list(recorded)
    );
}

#[test]
fn an_approval_takes_one_of_the_choices_it_offered() {
    let approval = asked("approval-answered-once", "approval.request");
    let session = approval.session.id.clone();
    assert!(session.starts_with("local-e36d3be5"));
    let frame = |choice: &str| {
        Ok(Some(
            json!({"jsonrpc": "2.0", "id": 7, "method": "approval.respond", "params": {
                "session_id": session, "prompt_id": "5c1c13ce8c354c329a25dafa08886e9f",
                "execution_generation": 1, "choice": choice,
            }}),
        ))
    };
    let table: &[(Value, Result<Option<Value>, String>)] = &[
        (json!({"kind": "allow"}), frame("once")),
        (scope("session"), frame("session")),
        (json!({"kind": "always"}), frame("always")),
        (scope("always"), frame("always")),
        (json!({"kind": "deny"}), frame("deny")),
        (json!({"kind": "leave"}), Ok(None)),
        (scope("forever"), Err(ONLY_APPROVED.into())),
        (
            pick("5c1c13ce8c354c329a25dafa08886e9f", &["once"], None),
            Err(ONLY_APPROVED.into()),
        ),
        (
            json!({"kind": "approve_plan", "acceptEdits": true}),
            Err(ONLY_APPROVED.into()),
        ),
        (json!({}), Err(ONLY_APPROVED.into())),
    ];
    for (decision, expected) in table {
        assert_eq!(
            sent(&approval, decision, Scope::Profile),
            *expected,
            "{decision}"
        );
    }

    // A protected file offers once and deny alone, and takes nothing more.
    let limited = asked("approval-limited-choice", "approval.request");
    let Ask::Approval(asked) = &limited.ask else {
        panic!("an approval");
    };
    assert_eq!(asked.choices, [Choice::Once, Choice::Deny]);
    for (decision, word) in [
        (json!({"kind": "allow"}), "once"),
        (json!({"kind": "deny"}), "deny"),
    ] {
        let frame = sent(&limited, &decision, Scope::Profile).unwrap().unwrap();
        assert_eq!(frame["params"]["choice"], word);
    }
    for decision in [json!({"kind": "always"}), scope("session"), scope("always")] {
        assert_eq!(
            sent(&limited, &decision, Scope::Profile),
            Err(NOT_OFFERED.into())
        );
    }
    assert_eq!(
        asked.respond(&limited.session, Choice::Session, Scope::Profile),
        Err(NotOffered(Choice::Session))
    );
    for choice in [Choice::Once, Choice::Session, Choice::Always, Choice::Deny] {
        assert_eq!(Choice::parse(choice.word()), Some(choice));
    }
    assert_eq!(Choice::parse("maybe"), None);
}

#[test]
fn a_set_of_questions_is_answered_by_a_result_frame_and_never_a_call() {
    let batch = asked("question-batch-answered-here", "clarify");
    let Ask::Batch(set) = &batch.ask else {
        panic!("a set of questions");
    };
    assert_eq!(set.request, "srq-be9ca8d8cb91");
    assert_eq!(batch.deadline_ms, None);
    let qids: Vec<&str> = set.questions.iter().map(|q| q.qid.as_str()).collect();
    assert_eq!(qids, ["q0", "q1"]);
    assert!(!set.questions[0].multi_select && set.questions[1].multi_select);
    let both = json!({"kind": "answer", "answers": {
        "q0": {"selected": ["green"]}, "q1": {"selected": ["small", "large"]},
    }});
    assert_eq!(
        sent(&batch, &both, Scope::Host),
        Ok(Some(
            json!({"jsonrpc": "2.0", "id": "srq-be9ca8d8cb91", "result": {
                "answers": {"q0": "green", "q1": r#"["small","large"]"#},
            }})
        ))
    );
    // One question answered: the other is left out, never sent as empty.
    assert_eq!(
        sent(&batch, &pick("q1", &["medium"], None), Scope::Profile),
        Ok(Some(
            json!({"jsonrpc": "2.0", "id": "srq-be9ca8d8cb91", "result": {
                "answers": {"q1": r#"["medium"]"#},
            }})
        ))
    );
    // Nothing answered sends nothing: a result without answers cancels the set.
    assert_eq!(
        sent(&batch, &pick("q0", &[], None), Scope::Profile),
        Err(EMPTY.into())
    );
    assert_eq!(
        sent(&batch, &json!({"kind": "leave"}), Scope::Profile),
        Ok(None)
    );
    assert_eq!(
        sent(&batch, &json!({"kind": "allow"}), Scope::Profile),
        Err(ONLY_ANSWERED.into())
    );
    assert!(matches!(
        batch.reply(&both, Scope::Profile),
        Ok(Some(Reply::Result(_)))
    ));
    // What another viewer already answered comes with a later reading.
    let mut params =
        fixture("question-batch-answered-here")["notes"]["request_frame"]["params"].clone();
    params["answers"] = json!({"q0": "green", "q1": null});
    let Some(Ask::Batch(read)) = Ask::batch("srq-1", "clarify", &params) else {
        panic!("a set of questions");
    };
    assert_eq!(
        read.questions[0].locked,
        Some(Locked::Answered("green".into()))
    );
    assert_eq!(read.questions[1].locked, Some(Locked::Skipped));
    // A password or a secret Hermes asks its viewers for is never a card.
    assert_eq!(Ask::batch("srq-2", "sudo", &params), None);
    assert_eq!(Ask::batch("srq-3", "secret", &params), None);
}

#[test]
fn a_card_offers_only_what_hermes_offers() {
    let question = asked("question-answered-here", "clarify.request").view(1_794_525_600_000);
    assert_eq!(question["id"], "hermes:local:hud-spike:local-f200386683510182cd45c8e38361a3bfecfa23934bacf51c394ec5b828f7ee9b:715a2db63d");
    assert_eq!(
        question["sessionId"],
        "hermes:hud-spike:local-f200386683510182cd45c8e38361a3bfecfa23934bacf51c394ec5b828f7ee9b"
    );
    assert_eq!(
        (&question["source"], &question["vendor"]),
        (&json!("hermes"), &json!("Hermes"))
    );
    assert_eq!(
        (&question["kind"], &question["answersQuestion"]),
        (&json!("question"), &json!(true))
    );
    assert_eq!(question["title"], "Which color should the widget be?");
    assert_eq!(question["createdAt"], "2026-11-12T23:20:00+00:00");
    assert_eq!(
        question["questions"],
        json!([{"id": "715a2db63d", "answerKey": "715a2db63d", "header": null, "multiSelect": false, "lock": null,
            "question": "Which color should the widget be?", "options": [
                {"label": "blue (Recommended)", "description": null},
                {"label": "green", "description": null}, {"label": "red", "description": null}]}])
    );
    assert_eq!(
        (&question["allowsAlways"], &question["suggestions"]),
        (&json!(false), &json!([]))
    );

    let open = asked("question-open-ended", "clarify.request").view(0);
    assert_eq!(open["questions"][0]["options"], json!([]));

    let set = asked("question-batch-answered-here", "clarify").view(0);
    let keys: Vec<&Value> = set["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| &q["answerKey"])
        .collect();
    assert_eq!(keys, ["q0", "q1"]);
    assert_eq!(set["questions"][1]["multiSelect"], true);
    assert_eq!(
        set["id"].as_str().unwrap().rsplit(':').next(),
        Some("srq-be9ca8d8cb91")
    );

    let approval = asked("approval-answered-once", "approval.request").view(0);
    assert_eq!(
        (&approval["kind"], &approval["answersQuestion"]),
        (&json!("permission"), &json!(false))
    );
    assert_eq!(approval["title"], "delete in root path");
    assert_eq!(
        approval["input"],
        "rm -r -- /Users/USER/project/owned/t-once"
    );
    assert_eq!(approval["detail"], approval["input"]);
    assert_eq!(approval["allowsAlways"], true);
    assert_eq!(approval["alwaysAllowRule"], "delete in root path");
    assert_eq!(
        approval["suggestions"],
        json!([{"type": "scope", "scope": "session"}, {"type": "scope", "scope": "always"}])
    );

    // Limited by Hermes: no lasting choice on the card at all.
    let limited = asked("approval-limited-choice", "approval.request").view(0);
    assert_eq!(limited["input"], "<write to AGENTS.md>");
    assert_eq!(limited["allowsAlways"], false);
    assert_eq!(limited["suggestions"], json!([]));
    assert_eq!(limited["alwaysAllowRule"], Value::Null);
}
