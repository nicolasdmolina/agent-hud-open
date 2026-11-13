//! The approval card's buttons, from the card to the frame Hermes got.
//! `tests/fixtures/hermes/approval-card-buttons.json` holds two cards as
//! the model shows them and the buttons the HUD draws on each, with the
//! decision each button sends. The HUD's own test
//! (`src/packages/hud/test/permission-choices.test.ts`) draws the buttons
//! from the same cards. This test checks the cards are the model's and
//! that each button's decision is the frame the recording sent.
use super::super::*;
use super::{fixture, local, spike};
use serde_json::Value;
use std::path::PathBuf;

fn contract() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hermes/approval-card-buttons.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The approval a recording asks, as the model holds it.
fn asked(name: &str) -> Pending {
    for entry in fixture(name)["clients"]["hud"].as_array().unwrap() {
        if entry["dir"] != "in" {
            continue;
        }
        if let Frame::Event(event) = frame(&entry["frame"]) {
            if event.kind == "approval.request" {
                let (ask, wait_ms) = Ask::prompt(event.payload).unwrap();
                return Pending {
                    session: local().session(&spike(), event.session),
                    ask,
                    wait_ms,
                };
            }
        }
    }
    panic!("{name} asks no approval");
}

/// The `approval.respond` frames the recording sent, in order.
fn recorded(name: &str) -> Vec<Value> {
    fixture(name)["clients"]["hud"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["dir"] == "out" && entry["frame"]["method"] == "approval.respond")
        .map(|entry| entry["frame"].clone())
        .collect()
}

/// The frame a button's decision sends, numbered as the recording's was.
fn sends(pending: &Pending, decision: &Value, id: &Value) -> Value {
    let Ok(Some(Reply::Call(call))) = pending.reply(decision, Scope::Profile) else {
        panic!("{decision} sends no call");
    };
    call.frame(id.as_u64().unwrap())
}

#[test]
fn the_cards_are_the_ones_the_model_shows() {
    let contract = contract();
    for (key, name) in [
        ("full", "approval-answered-once"),
        ("limited", "approval-limited-choice"),
    ] {
        let card = &contract[key];
        assert_eq!(card["recording"], name);
        assert_eq!(asked(name).view(0), card["card"], "{name}");
    }
}

#[test]
fn every_button_sends_the_answer_the_recording_sent() {
    let contract = contract();
    let full = &contract["full"]["buttons"];
    let labels: Vec<&str> = full
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        ["Deny", "Allow this session", "Always allow", "Allow once"]
    );
    // Each of the four was answered in a recording of its own; the button
    // sends exactly what that recording sent, and Hermes did what it says.
    for button in full.as_array().unwrap() {
        let name = button["recording"].as_str().unwrap();
        let [sent] = &recorded(name)[..] else {
            panic!("{name} answers once");
        };
        assert_eq!(
            sends(&asked(name), &button["decision"], &sent["id"]),
            *sent,
            "{}",
            button["label"]
        );
    }
    let words: Vec<Value> = full
        .as_array()
        .unwrap()
        .iter()
        .map(|b| recorded(b["recording"].as_str().unwrap())[0]["params"]["choice"].clone())
        .collect();
    assert_eq!(words, ["deny", "session", "always", "once"]);

    // A protected file: two buttons, and Hermes refused the session it did
    // not offer when the recording tried it.
    let limited = &contract["limited"];
    let name = limited["recording"].as_str().unwrap();
    let pending = asked(name);
    let buttons = limited["buttons"].as_array().unwrap();
    let labels: Vec<&str> = buttons
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Deny", "Allow once"]);
    let sent = recorded(name);
    assert_eq!(sent[0]["params"]["choice"], "session");
    assert_eq!(
        sends(&pending, &buttons[0]["decision"], &sent[1]["id"]),
        sent[1]
    );
    let once = sends(&pending, &buttons[1]["decision"], &sent[1]["id"]);
    let mut expected = sent[1].clone();
    expected["params"]["choice"] = "once".into();
    assert_eq!(once, expected);
    // What the full card's lasting buttons send, this card refuses.
    for button in &full.as_array().unwrap()[1..3] {
        assert!(pending.reply(&button["decision"], Scope::Profile).is_err());
    }
}
