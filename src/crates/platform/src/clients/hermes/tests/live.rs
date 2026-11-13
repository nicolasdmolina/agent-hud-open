//! Follows a real gateway for a while and prints what the connection
//! logged. Never run it against a Hermes home in use: point
//! `AGENT_HUD_HERMES_LAB` at a throwaway home with its own gateway, as
//! `FORK.md` says ("Hermes live channel").
//!
//! ```sh
//! AGENT_HUD_HERMES_LAB=/path/to/throwaway/.hermes AGENT_HUD_HERMES_LAB_SECONDS=60 \
//!   AGENT_HUD_HERMES_LAB_DATA=/folder/to/keep/the/log/in \
//!   cargo test -p agent-hud-platform --lib hermes::tests_unix::live -- --ignored --nocapture
//! ```
use super::super::{Observer, Pace};
use agent_hud_core::requests::Requests;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[test]
#[ignore = "needs a throwaway Hermes home with its own gateway"]
fn follows_a_real_gateway() {
    let Ok(home) = std::env::var("AGENT_HUD_HERMES_LAB") else {
        panic!("set AGENT_HUD_HERMES_LAB to a throwaway Hermes home");
    };
    let seconds = std::env::var("AGENT_HUD_HERMES_LAB_SECONDS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(30);
    // Where the log goes: a folder named to keep it, else a temporary one.
    let kept = std::env::var("AGENT_HUD_HERMES_LAB_DATA")
        .ok()
        .map(PathBuf::from);
    let scratch = tempfile::tempdir().unwrap();
    let data = kept.as_deref().unwrap_or(scratch.path());
    let requests = Requests::default();
    let observer = Observer::start_at(
        PathBuf::from(home),
        data,
        requests.clone(),
        Arc::new(|| {}),
        Pace {
            trace: true,
            ..Pace::default()
        },
    )
    .unwrap();
    // `AGENT_HUD_HERMES_LAB_ANSWER=1` answers each card a second after it
    // shows, as a person would on the HUD: the first option of a question
    // (or the word "typed" where it offers none), "allow" for an approval.
    let answers = std::env::var("AGENT_HUD_HERMES_LAB_ANSWER").is_ok();
    let mut seen: std::collections::HashMap<String, std::time::Instant> = Default::default();
    let log = data.join("hermes-gateway.log");
    let mut shown = 0;
    for _ in 0..seconds * 10 {
        std::thread::sleep(Duration::from_millis(100));
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        print!("{}", &text[shown..]);
        shown = text.len();
        let now = chrono::Utc::now().timestamp_millis();
        for view in requests.views(now).iter().filter(|_| answers) {
            let id = view["id"].as_str().unwrap().to_owned();
            let first = *seen
                .entry(id.clone())
                .or_insert_with(std::time::Instant::now);
            if first.elapsed() < Duration::from_secs(1) {
                continue;
            }
            let decision = if view["kind"] == "question" {
                let mut written = serde_json::Map::new();
                for question in view["questions"].as_array().unwrap() {
                    let key = question["answerKey"].as_str().unwrap().to_owned();
                    let answer = match question["options"][0]["label"].as_str() {
                        Some(label) => serde_json::json!({"selected": [label]}),
                        None => serde_json::json!({"custom": "typed"}),
                    };
                    written.insert(key, answer);
                }
                serde_json::json!({"kind": "answer", "answers": written})
            } else {
                serde_json::json!({"kind": "allow"})
            };
            println!("answering {id}: {:?}", requests.answer(&id, &decision, now));
            seen.remove(&id);
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    for row in requests.deliveries(now) {
        println!("delivery {} {} {}", row["id"], row["state"], row["reason"]);
    }
    for view in requests.views(now) {
        println!("card {} {} {}", view["id"], view["kind"], view["title"]);
    }
    drop(observer);
}
