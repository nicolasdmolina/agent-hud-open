//! The hook's answer as the Windows build has it (Agent HUD Fork, ticket
//! 66, round 8; the round 7 verifier's lint reproduction). `answer.rs` is
//! shared code, and each listener calls what it needs of it. A part only
//! the Unix listener calls is dead code in the Windows library, where
//! warnings are errors, and this Mac cannot compile that library to see.
//!
//! So the file is compiled here a second time, alone, with dead code an
//! error, and called only the way shared code and the Windows listener
//! call it: `wait_for_answer` builds either kind of answer, and
//! `hooks/windows.rs` hands it over with the one call written out below
//! (`delivered_tests.rs` holds that file to the same call). A part of the
//! answer that Windows does not reach stops this test from compiling.
#![deny(dead_code)]
use agent_hud_core::requests::Requests;
use serde_json::json;

#[allow(clippy::duplicate_mod)]
#[path = "answer.rs"]
mod answer;

/// `answer.rs` reads the clock of the module it sits in.
fn now_ms() -> i64 {
    0
}

/// Either kind of answer, as `callback::wait_for_answer` builds them.
fn answers() -> [answer::Answer; 2] {
    let allow = json!({"decision": "allow"});
    let automatic = answer::Answer::automatic(allow.clone(), "claude", "s1", Some("toolu_1"));
    [automatic, answer::Answer::by_hand(allow)]
}

#[test]
fn the_windows_listener_leaves_no_part_of_the_answer_unused() {
    let requests = Requests::default();
    for answer in answers() {
        // The call in `hooks/windows.rs`.
        let reply = answer.deliver(&requests, |_| false);
        assert_eq!(reply["decision"], "allow");
    }
    // And it tells core of no approval.
    assert!(requests.auto_approved(now_ms()).is_empty());
}
