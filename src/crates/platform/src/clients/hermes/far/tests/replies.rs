//! What comes back: bounded, one line, and asked once more when empty.
use super::{
    super::{Far, Silent},
    fixture::{text, FarLab, Manner},
};
use agent_hud_agents::hermes::gateway::far::{Quiet, Request};
use serde_json::json;

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn an_answer_over_the_size_bound_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Floods);
    let far = Far::default();
    let told = far.exchange(&lab.link, text(&home), Request::Identify);
    println!("refused after {:?}", told.took);
    assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::TooLong));
    assert_eq!(gate.seen().len(), 1);
    assert_eq!(lab.relays(&home), 0);
    // An answer just under the bound comes back whole.
    let other = lab.profile("big");
    let filler = "x".repeat(500 * 1024);
    let _big = lab.gate(&other, Manner::Says(json!({"filler": filler})));
    let told = far.exchange(&lab.link, text(&other), Request::Identify);
    let told = told.said.expect("an answer under the bound");
    assert_eq!(
        told.value()["filler"].as_str().map(str::len),
        Some(500 * 1024)
    );
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn an_empty_reply_is_asked_once_more() {
    let Some(lab) = FarLab::start() else { return };
    let far = Far::default();
    // Empty once, as measured on a real machine: the second try answers.
    let home = lab.root();
    let gate = lab.gate(&home, Manner::EmptyThen(1, json!({"who": "second try"})));
    let told = far.exchange(&lab.link, text(&home), Request::Identify);
    assert_eq!(told.tries, 2);
    assert_eq!(told.said.unwrap().value()["who"], "second try");
    assert_eq!(gate.seen().len(), 2);
    // Empty every time: asked twice and no more.
    let other = lab.profile("mute");
    let mute = lab.gate(&other, Manner::EmptyThen(usize::MAX, json!({})));
    let told = far.exchange(&lab.link, text(&other), Request::Identify);
    assert_eq!(told.tries, 2);
    assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::Empty));
    assert_eq!(mute.seen().len(), 2);
    // An answer that is no JSON line, and one cut before its end.
    let odd = lab.profile("odd");
    let _odd = lab.gate(&odd, Manner::Says(json!("not an object")));
    let told = far.exchange(&lab.link, text(&odd), Request::Identify);
    assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::Garbled));
    assert_eq!(told.tries, 1);
    // No gateway there at all.
    let none = lab.profile("none");
    let told = far.exchange(&lab.link, text(&none), Request::Identify);
    assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::Absent));
    let gone = lab.dir.join("far").join("no-such-home");
    let told = far.exchange(&lab.link, text(&gone), Request::Identify);
    assert_eq!(told.said.unwrap_err(), Silent::Relay(Quiet::Absent));
}
