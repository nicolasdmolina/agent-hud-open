//! What one poll costs the model in the worst case Hermes allows: one
//! session whose stream is full, 512 events of about 4 MiB together. The
//! model reads the whole of it again on every poll, so this is the number
//! that says whether that is affordable or the reads must become
//! incremental. Run it for the figures with
//! `cargo test --release -p agent-hud-agents --lib hermes::gateway::tests::cost -- --nocapture`.
use super::super::*;
use super::fake::{identified, names, Hermes};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const S: &str = "local-full";

/// A Hermes whose one session holds 512 events of `bytes` each, a question
/// that waits among them.
fn full(bytes: usize) -> Hermes {
    let mut hermes = Hermes::default();
    let profile = Profile::named("verify");
    let filler = "x".repeat(bytes);
    for index in 0..512 {
        if index == 300 {
            let body = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
                "question": "Which one?", "choices": ["a", "b"], "multi_select": false});
            hermes.publish(&profile, S, "clarify.request", 1, body);
        } else {
            let body = json!({"name": "terminal", "result": filler});
            hermes.publish(&profile, S, "tool.complete", 1, body);
        }
    }
    hermes
}

#[test]
fn a_full_stream_is_folded_again_on_every_poll_within_budget() {
    // 512 events of 8 KiB are 4 MiB, which is all Hermes keeps of a stream.
    let mut hermes = full(8 * 1024);
    let mut gateway = Gateway::new(GatewayId::new("local"));
    identified(&mut gateway, "one");
    let at = gateway.session(&Profile::named("verify"), S);
    let cursor = Some(Cursor {
        epoch: "epoch-1".into(),
        seq: 0,
    });
    let read = Step::Since {
        session: at,
        cursor,
    };
    let reply = serde_json::to_string(&hermes.answer(&read).unwrap()).unwrap();
    assert!(reply.len() > 4 * 1024 * 1024, "{} bytes", reply.len());

    // What the connection does with the reply: parse it.
    let rounds = 20;
    let started = Instant::now();
    for _ in 0..rounds {
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["events"].as_array().unwrap().len(), 512);
    }
    let parse = started.elapsed() / rounds;

    // What the model does with it: fold the 512 events and settle the
    // cards. Timed from the parsed reply going in to the effects coming out.
    let parsed: Value = serde_json::from_str(&reply).unwrap();
    let where_it_is =
        json!({"replay_epoch": "epoch-1", "latest_seq": 512, "snapshot_required": true});
    let mut fold = Duration::ZERO;
    for round in 0..rounds {
        let mut poll = gateway.poll().unwrap();
        for step in poll.steps() {
            poll.answer(&step, &hermes.answer(&step).unwrap());
        }
        let learn = poll.steps().remove(0);
        poll.answer(&learn, &where_it_is);
        assert_eq!(poll.steps(), std::slice::from_ref(&read));
        let started = Instant::now();
        poll.answer(&read, &parsed);
        let effects = gateway.apply(Input::Polled(poll), 0);
        fold += started.elapsed();
        assert_eq!(effects.len(), usize::from(round == 0));
    }
    let fold = fold / rounds;
    assert_eq!(names(&gateway), ["p1"]);
    println!(
        "full stream: {} bytes, 512 events: parse {parse:?}, fold and settle {fold:?} a poll",
        reply.len()
    );
    // A poll comes round every second. Even an unoptimised build must
    // leave most of that second free.
    assert!(fold.as_millis() < 250, "fold took {fold:?}");
    assert!(parse.as_millis() < 750, "parse took {parse:?}");
}
