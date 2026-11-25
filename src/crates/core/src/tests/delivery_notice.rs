//! A receipt of an answer that did not arrive names its session by the
//! session's own id, as its card does (ticket 66, round 8; the round 7
//! verifier's probe). The HUD shows the receipt's reason under a card of
//! that session by comparing the two ids. Round 7 wrote the receipt's
//! session as `source:session`, core's own key for it, and the notice
//! vanished under an open card. The fixture is what the snapshot carries;
//! the HUD's `test/request-delivery.test.ts` draws the notice from it.
use super::auto_approved_sound::{Lab, THREAD};
use super::*;

fn fixture() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/request-delivery-notice.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_receipt_names_its_session_as_the_card_of_that_session_does() {
    let fixture = fixture();
    // With Approve automatically off and on: the receipt is the same.
    for automatic in [false, true] {
        let mut lab = Lab::new(automatic);
        let requests = lab.core.requests();
        // A question always shows its card.
        assert!(!lab.card("codex-desktop-question:open", "question"));
        let why = Some("fixture refusal".to_owned());
        requests.note_native_delivery("earlier-request", "codex", THREAD, "failed", why, lab.now);
        let snapshot = lab.core.request("request_snapshot", json!({})).unwrap();
        let card = &snapshot["approvals"][0];
        let receipt = &snapshot["requestDeliveries"][0];
        assert_eq!(card["sessionId"], THREAD);
        assert_eq!(
            receipt["sessionId"], card["sessionId"],
            "the receipt and the card name the session differently"
        );
        for key in ["source", "sessionId"] {
            assert_eq!(card[key], fixture["card"][key], "card {key}");
        }
        for key in ["id", "source", "sessionId", "state", "reason"] {
            assert_eq!(receipt[key], fixture["receipt"][key], "receipt {key}");
        }
        assert!(receipt["at"].is_string());
    }
}
