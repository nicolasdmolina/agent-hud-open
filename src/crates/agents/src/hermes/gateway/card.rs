//! The card a pending request shows, in the one shape every request view
//! has (`Approval` in the protocol package). The card offers only what
//! Hermes offers.
use super::ask::{Approval, Ask, Choice, Locked, Pending};
use super::identity::Profile;
use crate::hermes::AGENT;
use crate::l10n;
use crate::naming::clients::client_name;
use serde_json::{json, Value};

impl Pending {
    /// The card, first seen at `now_ms`. `machine` is the label of the far
    /// machine the session runs on, as its rows carry it; None on this one,
    /// and then the card has no `machine` at all.
    pub fn view(&self, now_ms: i64, machine: Option<&str>) -> Value {
        let mut view = json!({
            "id": self.id(),
            "source": AGENT.id,
            // Who asks, as its sessions are named: the profile's label.
            "vendor": label(&self.session.profile),
            "sessionId": self.session.hud_id(),
            "createdAt": crate::iso(now_ms),
            "cwd": null,
            "plan": null,
            "answersPlan": false,
        });
        if let Some(machine) = machine {
            // A card can show before its session has a row.
            view["machine"] = json!(machine);
        }
        let shown = match &self.ask {
            Ask::Question(question) => {
                let mut card = questions(vec![question_item(
                    &question.prompt.id,
                    &question.text,
                    &question.choices,
                    question.multi_select,
                )]);
                // No answer a viewer sends skips one question everywhere:
                // Hermes's own skip value is internal, and the empty answer
                // its contract calls a skip is, where the gateway runs the
                // turn itself, a timeout that also drops the questions still
                // to come in the call (`run_turn_runner_clarify_delivery.py`,
                // `run_turn_runner.py`). So the card offers no Skip.
                card["canSkip"] = json!(false);
                card
            }
            Ask::Batch(batch) => questions(
                batch
                    .questions
                    .iter()
                    .map(|q| {
                        let mut item = question_item(&q.qid, &q.text, &q.choices, q.multi_select);
                        item["lock"] = lock(q.locked.as_ref());
                        item
                    })
                    .collect(),
            ),
            Ask::Approval(approval) => permission(approval),
        };
        for (key, value) in shown.as_object().into_iter().flatten() {
            view[key] = value.clone();
        }
        view
    }
}

/// The label the sessions of `profile` go by (`Hermes Agent · <profile>`).
fn label(profile: &Profile) -> String {
    let named = match profile {
        Profile::Default => None,
        Profile::Named(name) => Some(name.as_str()),
    };
    client_name(AGENT.id, named).unwrap_or_else(|| AGENT.name.to_owned())
}

fn question_item(key: &str, text: &str, choices: &[String], multi_select: bool) -> Value {
    json!({
        "id": key,
        "answerKey": key,
        "question": text,
        "header": null,
        "options": choices
            .iter()
            .map(|label| json!({"label": label, "description": null}))
            .collect::<Vec<_>>(),
        "multiSelect": multi_select,
        "lock": null,
    })
}

/// What another viewer already did with one question of a set: answered
/// it, or skipped it. An answer given on this card replaces it; a question
/// left alone keeps it.
fn lock(locked: Option<&Locked>) -> Value {
    match locked {
        Some(Locked::Answered(answer)) => json!({"state": "answered", "answer": answer}),
        Some(Locked::Skipped) => json!({"state": "skipped"}),
        None => Value::Null,
    }
}

fn questions(items: Vec<Value>) -> Value {
    let summary = items
        .first()
        .map_or(Value::Null, |item| item["question"].clone());
    json!({
        "kind": "question",
        "toolName": "clarify",
        "title": summary,
        "summary": summary,
        "input": json!({"questions": items}).to_string(),
        "detail": null,
        "allowsAlways": false,
        "suggestions": [],
        "questions": items,
        "answersQuestion": true,
    })
}

/// An approval's card. Allowing once and denying are its two plain
/// buttons; the lasting choices Hermes offers are its `suggestions`, one
/// `{type: "scope", scope, label}` each, which the card draws as a button
/// of its own, and a decision `always` names the one chosen (none named is
/// `always` itself).
fn permission(approval: &Approval) -> Value {
    let lasting: Vec<Value> = approval
        .choices
        .iter()
        .filter_map(|choice| {
            let label = match choice {
                Choice::Session => l10n::text("本会话内允许", "Allow this session"),
                Choice::Always => l10n::text("总是允许", "Always allow"),
                Choice::Once | Choice::Deny => return None,
            };
            Some(json!({"type": "scope", "scope": choice.word(), "label": label}))
        })
        .collect();
    let always = approval.choices.contains(&Choice::Always);
    let title = if approval.description.is_empty() {
        &approval.command
    } else {
        &approval.description
    };
    json!({
        "kind": "permission",
        "toolName": "",
        "title": title,
        "summary": title,
        "input": approval.command,
        "detail": approval.command,
        "allowsAlways": !lasting.is_empty(),
        // What always allowing adds to the profile's own config.
        "alwaysAllowRule": always.then_some(&approval.description),
        "suggestions": lasting,
        "questions": [],
        "answersQuestion": false,
    })
}
