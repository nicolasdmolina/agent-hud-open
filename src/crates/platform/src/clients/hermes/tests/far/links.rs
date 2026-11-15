//! What becomes of a far machine's cards when its gateway restarts or
//! the link is lost, and what each socket costs once the link is up.
use super::{
    super::{fake::Fake, marked, plain},
    far_lab, until, FarLab, Standing, INSTALL,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn following(instance: &str) -> Option<(FarLab, Fake)> {
    let mut far = far_lab()?;
    let gateway = far.gateway(instance);
    far.follow();
    far.followed();
    Some((far, gateway))
}

fn pick(card: &Value, choice: &str) -> Value {
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    json!({"kind": "answer", "answers": {key: {"selected": [choice]}}})
}

/// Ends the master of the link that is open, as a sleep or a dropped
/// network does: with no word to anyone.
fn kill_master(far: &FarLab) {
    let (link, _) = far.inner().route.link().expect("a link");
    let pid = link.master_pid().expect("its master");
    // SAFETY: a signal to the master this test's observer started.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_gateway_restarted_mid_question_is_followed_again_on_its_new_port() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    gateway.play("gateway-restart-hard", Some("approval.request"));
    let before = far.card();
    let old_port = gateway.world().identify.as_ref().unwrap()["result"]["api_origin"].clone();
    assert_eq!(far.inner().route.forwards_made(), 1);

    // Killed: the request may still wait, so the card stays, and says
    // that its gateway cannot be reached.
    gateway.kill();
    until("the card is marked", || far.cards().iter().all(marked));
    assert_eq!(far.cards().len(), 1);
    assert_eq!(plain(far.card()), before);

    // Back as another instance, on another port: the old card goes, and
    // the same observer shows and answers what the new one asks.
    let gateway = far.gateway("instance-2");
    let new_port = gateway.world().identify.as_ref().unwrap()["result"]["api_origin"].clone();
    assert_ne!(old_port, new_port);
    until("the card of the old instance goes", || {
        far.cards().is_empty()
    });
    assert!(far.log().contains("why=GatewayRestarted"), "{}", far.log());
    gateway.play("question-answered-here", Some("clarify.request"));
    let card = far.card();
    assert!(!marked(&card));
    let id = card["id"].as_str().unwrap().to_owned();
    far.answer(&id, &pick(&card, "green"));
    assert_eq!(far.delivery(&id)["state"], "confirmed");
    until("the card goes", || far.cards().is_empty());
    // One forward for each instance, on the one link, with one login.
    assert_eq!(far.inner().route.forwards_made(), 2);
    assert_eq!(far.logins(), 2, "the lab's own check, and the one link");
    assert_eq!(far.log().matches("link up").count(), 1);
    // The new instance said who it is before it was polled.
    assert_eq!(gateway.world().calls("profiles.list").len(), 1);
    let world = gateway.world();
    let first = &world.received[0].1;
    assert_eq!(first["method"], "profiles.list", "{first}");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_killed_master_marks_the_card_and_the_link_comes_back_to_answer_it() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    gateway.play("question-answered-here", Some("clarify.request"));
    let before = far.card();
    let id = before["id"].as_str().unwrap().to_owned();
    let shows = |far: &FarLab| {
        let all = far.inner().followers().unwrap();
        let shown = all.set()[0]
            .shows
            .load(std::sync::atomic::Ordering::Acquire);
        shown
    };

    kill_master(&far);
    until("the card is marked", || far.cards().iter().all(marked));
    // The same card, changed in place: it takes no answer while marked.
    assert_eq!(plain(far.card()), before);
    let refused = far
        .requests
        .answer(&id, &pick(&before, "green"), crate::hooks::now_ms());
    assert!(refused.is_err(), "a marked card took an answer");
    assert!(gateway.world().calls("clarify.respond").is_empty());

    // The link comes back by itself, the gateway says who it is again,
    // and the first poll finds the request still waiting: the mark goes.
    until("the mark clears", || !far.cards().iter().any(marked));
    assert_eq!(far.card(), before, "the very same card");
    assert_eq!(shows(&far), 1, "registered once, so one needs-you");
    assert_eq!(far.log().matches("link up").count(), 2);
    assert_eq!(far.inner().route.forwards_made(), 2, "forwarded again");
    // Asked who it is once before it was followed, and once more on the
    // first socket over the new link.
    assert_eq!(
        gateway.world().calls("profiles.list").len(),
        2,
        "asked again"
    );
    far.answer(&id, &pick(&before, "green"));
    assert_eq!(far.delivery(&id)["state"], "confirmed");
    until("the card goes", || far.cards().is_empty());
    assert_eq!(far.standing(), Standing::Connected(far.followed()));
}

/// Kills the master and keeps the far machine out of reach for `down`:
/// the card is there, marked, the whole time, and is found waiting when
/// the machine can be reached again.
pub(in super::super) fn held_down(down: Duration) {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    gateway.play("question-answered-here", Some("clarify.request"));
    let before = far.card();
    // Every new master is sent to a port nobody listens on.
    let config = far.lab.config_text();
    let closed = super::lab::closed_port();
    let nowhere = config.replace(&format!("Port {}", far.lab.port), &format!("Port {closed}"));
    assert_ne!(config, nowhere);
    std::fs::write(far.lab.at("config"), nowhere).unwrap();
    kill_master(&far);
    until("the card is marked", || far.cards().iter().all(marked));
    until("the machine is said to be out of reach", || {
        matches!(far.standing(), Standing::CannotReach(_))
    });
    let began = Instant::now();
    let mut looks = 0;
    while began.elapsed() < down {
        let cards = far.cards();
        assert_eq!(cards.len(), 1, "after {:?}", began.elapsed());
        assert!(marked(&cards[0]), "after {:?}", began.elapsed());
        assert_eq!(plain(cards[0].clone()), before);
        assert!(matches!(far.standing(), Standing::CannotReach(_)));
        looks += 1;
        std::thread::sleep(Duration::from_millis(250));
    }
    let tries = far.log().matches("link not open").count();
    println!(
        "held down {:?}: the card was there and marked at each of {looks} looks; \
         standing {:?}; reasons logged for the link not opening: {tries}",
        began.elapsed(),
        far.standing()
    );
    assert!(gateway.world().calls("clarify.respond").is_empty());
    // Reachable again: the keeper never gave up.
    std::fs::write(far.lab.at("config"), config).unwrap();
    until("the mark clears", || !far.cards().iter().any(marked));
    assert_eq!(far.card(), before);
    let id = before["id"].as_str().unwrap().to_owned();
    far.answer(&id, &pick(&before, "green"));
    assert_eq!(far.delivery(&id)["state"], "confirmed");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_machine_held_out_of_reach_keeps_its_card_marked() {
    held_down(Duration::from_secs(12));
}

/// What is asked of the route for each socket, in order: the forward is
/// in place, then the ticket, then the dial. And what a snapshot and an
/// answer cost: a ticket and a socket each, on the forward there is, with
/// no login.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_snapshot_and_an_answer_each_take_a_new_ticket_on_the_existing_forward() {
    let Some((far, gateway)) = following("instance-1") else {
        return;
    };
    until("the polls run", || {
        !gateway.world().calls("session.active_list").is_empty()
    });
    let counts = |far: &FarLab, gateway: &Fake| {
        let world = gateway.world();
        let route = &far.inner().route;
        (
            world.granted.len(),
            world.sockets,
            route.forwards_made(),
            far.logins(),
        )
    };
    // The first socket, which said who the machine is, and the polls'.
    let start = counts(&far, &gateway);
    assert_eq!(start, (2, 2, 1, 2));

    // A request no read of the events shows: found by a snapshot.
    {
        let mut world = gateway.world();
        let stream = world.stream("s-hidden");
        stream.epoch = "epoch-1".into();
        stream.running = true;
        stream
            .prompts
            .push(json!({"kind": "clarify", "prompt_id": "p-hidden",
            "execution_generation": 1, "question": "Which one?", "choices": ["one", "two"],
            "multi_select": false, "timeout_s": 600}));
    }
    let card = far.card();
    let resumed = gateway.world().calls("session.resume");
    assert_eq!(resumed.len(), 1, "one snapshot");
    until("the snapshot's socket is done with", || {
        gateway.world().visit_over()
    });
    assert_eq!(counts(&far, &gateway), (3, 3, 1, 2), "a snapshot");

    let id = card["id"].as_str().unwrap().to_owned();
    far.answer(&id, &pick(&card, "one"));
    assert_eq!(far.delivery(&id)["state"], "confirmed");
    until("the card goes", || far.cards().is_empty());
    let answered = gateway.world().calls("clarify.respond");
    assert_eq!(answered.len(), 1);
    let after = counts(&far, &gateway);
    assert_eq!((after.2, after.3), (1, 2), "the one forward, no login");
    // The answer's own ticket and socket; a snapshot after it may be one more.
    assert!(after.0 >= 4 && after.0 == after.1, "{after:?}");
    assert_eq!(far.log().matches("link up").count(), 1);

    // Every socket: prepared, then its ticket, then its dial, as each
    // thread that opens one asked for them.
    let steps = far.inner().route.all_steps();
    let of = |step: &str| steps.iter().filter(|(said, _)| *said == step).count();
    assert_eq!(of("ticket"), after.0, "{steps:?}");
    assert_eq!(of("dial"), after.1, "{steps:?}");
    let mut last = std::collections::HashMap::new();
    for (step, thread) in steps.iter().filter(|(step, _)| *step != "identify") {
        let before = last.insert(*thread, *step).unwrap_or("");
        let follows = match *step {
            "prepare" => matches!(before, "" | "dial"),
            "ticket" => before == "prepare",
            "dial" => before == "ticket",
            _ => false,
        };
        assert!(follows, "{step} after {before:?}: {steps:?}");
    }
    assert!(gateway
        .world()
        .granted
        .iter()
        .all(|ticket| !far.log().contains(ticket)));
    assert_eq!(gateway.world().install, Some(INSTALL.into()));
}
