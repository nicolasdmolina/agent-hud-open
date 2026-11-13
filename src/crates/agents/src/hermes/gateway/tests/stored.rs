//! A machine key read back from a file Agent HUD wrote (`StoredKey`), and
//! the machine label a card is given. A stored key names the same sessions
//! a verified key names, and nothing in the model can be built from it.
use super::super::*;
use super::fake::{identified, poll, Hermes};
use super::{far, FAR_HOME, INSTALL};
use serde_json::json;

const S: &str = "20261104_101500_a1b2c3";

#[test]
fn a_stored_key_is_exactly_what_a_key_is_written_as() {
    let key = machine_key(Some(INSTALL), FAR_HOME).unwrap();
    assert_eq!(StoredKey::parse(key.as_str()), Some(key.stored()));
    assert_eq!(key.stored().as_str(), "aaeda597d860920c");
    for (case, text) in [
        ("empty", ""),
        ("15 characters", "aaeda597d860920"),
        ("17 characters", "aaeda597d860920c0"),
        ("upper case", "AAEDA597D860920C"),
        ("one upper case", "aaeda597d860920C"),
        ("not hex", "aaeda597d860920g"),
        ("a line end", "aaeda597d860920\n"),
        ("a space before", " aaeda597d860920c"),
        ("a slash", "aaeda597/860920c"),
        ("a whole install id", INSTALL),
        ("local", "local"),
    ] {
        assert_eq!(StoredKey::parse(text), None, "{case}");
    }
}

#[test]
fn a_stored_key_names_the_sessions_its_verified_key_names() {
    let key = machine_key(Some(INSTALL), FAR_HOME).unwrap();
    let stored = StoredKey::parse(key.as_str()).unwrap();
    let machine = Machine::Far(key);
    for profile in [Profile::Default, Profile::named("work").unwrap()] {
        for id in [S, "agent:main:signal:group:Ab/Cd=="] {
            let live = hud_session_id(&machine, &profile, id);
            assert!(live.is_some(), "{profile:?}");
            assert_eq!(stored_session_id(&stored, &profile, id), live);
        }
        // An id that can be no session's is refused the same way.
        for id in ["", "a/b:c"] {
            assert_eq!(stored_session_id(&stored, &profile, id), None);
        }
    }
    assert_eq!(
        stored_session_id(&stored, &Profile::Default, S).unwrap(),
        format!("hermes:aaeda597d860920c/default:{S}")
    );
    // A stored key never names one of this machine's sessions.
    let local = hud_session_id(&Machine::This, &Profile::Default, S).unwrap();
    assert_ne!(
        stored_session_id(&stored, &Profile::Default, S).unwrap(),
        local
    );
}

#[test]
fn a_card_carries_the_machine_label_it_is_given_and_none_without() {
    let verify = Profile::named("verify").unwrap();
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), verify.clone()));
    identified(&mut gateway, "instance-1");
    let mut hermes = Hermes::default();
    let asked = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
        "question": "Synthetic question", "choices": ["a", "b"], "multi_select": false});
    hermes.publish(&verify, S, "clarify.request", 1, asked);
    let effects = poll(&mut gateway, &mut hermes, 0);
    let [Effect::Open(pending)] = &effects[..] else {
        panic!("{effects:?}");
    };
    let shown = pending.view(0, Some("Mac mini"));
    assert_eq!(shown["machine"], "Mac mini");
    assert_eq!(
        shown["sessionId"],
        format!("hermes:aaeda597d860920c/verify:{S}")
    );
    // Without a label the card is what it was: no `machine` at all.
    let mut plain = pending.view(0, None);
    assert!(plain.get("machine").is_none(), "{plain}");
    plain["machine"] = json!("Mac mini");
    assert_eq!(plain, shown);
}
