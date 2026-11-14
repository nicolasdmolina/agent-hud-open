//! The independent verifier's probes from the first review of ticket 30
//! (`evidence/30/verifier-files/review30.rs`), as written but for where
//! they are registered. The first failed on the reviewed head: a gateway
//! that restarts after its follower began asking another of its homes
//! left the old instance's forward open. The other four passed there and
//! are kept as they are: every follower of a machine loses a dead link,
//! the far pace is the default, a session outside the live list is never
//! resumed, and an answer taken before a link was lost is never sent
//! over the next.
use super::{
    super::{fake::Fake, marked},
    far_lab, until, INSTALL,
};
use std::sync::atomic::Ordering;

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn review30_every_follower_loses_a_dead_link() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let one = Fake::serving(&far.root, "instance-a", &["default"]);
    let two = Fake::serving(&far.root, "instance-b", &["reports"]);
    for gateway in [&one, &two] {
        gateway.world().install = Some(INSTALL.into());
    }
    one.asks_quietly("default", "s-one", "p-one");
    two.asks_quietly("reports", "s-two", "p-two");
    far.follow();
    far.followed();
    until("two cards", || far.cards().len() == 2);
    let old = far.cards();
    let lost_before = far.log().matches("connection failed reason=").count();
    let config = far.lab.config_text();
    let closed = crate::ssh_link::unix::tests::lab::closed_port();
    std::fs::write(
        far.lab.at("config"),
        config.replace(&format!("Port {}", far.lab.port), &format!("Port {closed}")),
    )
    .unwrap();
    let followers = far.inner().followers().unwrap();
    let (link, _) = far.inner().route.link().unwrap();
    unsafe {
        libc::kill(link.master_pid().unwrap() as libc::pid_t, libc::SIGKILL);
    }
    until("both marked", || {
        far.cards().len() == 2 && far.cards().iter().all(marked)
    });
    until("each follower reports Lost", || {
        far.log().matches("connection failed reason=").count() >= lost_before + 2
    });
    let set = followers.set();
    assert_eq!(set.len(), 2);
    assert!(set.iter().all(|f| !f.gone.load(Ordering::Acquire)));
    assert!(set.iter().all(|f| !matches!(
        f.model().gateway.connection(),
        agent_hud_agents::hermes::gateway::Connection::Ready(_)
    )));
    assert!(one.world().calls("clarify.respond").is_empty());
    assert!(two.world().calls("clarify.respond").is_empty());
    println!(
        "TRACE dead link followers=2 marked_cards=2 gone=false waiting_sessions={} prior_cards={}",
        far.requests.native_waiting_sessions()["hermes"].len(),
        old.len()
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn review30_restart_after_anchor_retirement_cancels_the_old_forward() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = Fake::serving(&far.root, "instance-before", &["default", "reports"]);
    gateway.world().install = Some(INSTALL.into());
    far.follow();
    far.followed();
    gateway.asks_quietly("reports", "s-reports", "p-before");
    far.card();
    gateway.serve(&["reports"]);
    until("the retired default is no longer polled", || {
        let followers = far.inner().followers().unwrap();
        let set = followers.set();
        set.len() == 1
            && set[0].asked()
                == agent_hud_agents::hermes::gateway::Profile::named("reports").unwrap()
    });
    let before_origin = gateway.world().identify.as_ref().unwrap()["result"]["api_origin"].clone();
    gateway.kill();
    let next = Fake::serving(&far.root, "instance-after", &["reports"]);
    next.world().install = Some(INSTALL.into());
    next.asks_quietly("reports", "s-next", "p-after");
    until("the new gateway's card", || {
        far.cards()
            .iter()
            .any(|c| c["id"].as_str().is_some_and(|id| id.ends_with(":p-after")))
    });
    let (link, _) = far.inner().route.link().unwrap();
    let pid = link.master_pid().unwrap().to_string();
    let output = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fn"])
        .output()
        .unwrap();
    let listeners: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|s| s.strip_prefix('n'))
        .map(str::to_owned)
        .collect();
    let count = listeners.len();
    println!(
        "TRACE remote before={} after={} local_listeners={listeners:?}",
        before_origin,
        next.world().identify.as_ref().unwrap()["result"]["api_origin"]
    );
    println!(
        "TRACE restart after root retirement forwards_made={} current_listeners={count}",
        far.inner().route.forwards_made()
    );
    assert_eq!(count, 1, "the replaced gateway's forward must be cancelled");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn review30_far_pace_keeps_every_default() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let expected = super::super::super::Pace::default();
    far.follow_at(expected);
    let got = far.inner().pace;
    assert_eq!(got.watch, expected.watch);
    assert_eq!(got.gap, expected.gap);
    assert_eq!(got.discover, expected.discover);
    assert_eq!(got.retry, expected.retry);
    println!(
        "TRACE far watch_ms={} gap_ms={} discovery_ms={}",
        got.watch.as_millis(),
        got.gap.as_millis(),
        got.discover.unwrap().as_millis()
    );
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn review30_never_resumes_a_session_outside_the_live_list() {
    use serde_json::json;
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = Fake::serving(&far.root, "instance-1", &["default"]);
    gateway.world().install = Some(INSTALL.into());
    gateway.world().unlisted.push("s-unlisted".into());
    for (session, prompt) in [("s-listed", "p-listed"), ("s-unlisted", "p-unlisted")] {
        let mut world = gateway.world();
        let stream = world.stream_in("default", session);
        stream.epoch = "epoch-1".into();
        stream.running = true;
        stream.prompts.push(json!({"kind":"clarify","prompt_id":prompt,"execution_generation":1,"question":"Which?","choices":["one","two"],"multi_select":false,"timeout_s":600}));
    }
    far.follow();
    far.followed();
    until("listed session snapshot", || {
        far.cards()
            .iter()
            .any(|c| c["id"].as_str().is_some_and(|id| id.ends_with(":p-listed")))
    });
    until("several polls", || gateway.world().listed.len() > 4);
    let world = gateway.world();
    let calls = world.calls("session.resume");
    let unlisted = calls
        .iter()
        .filter(|(_, frame)| frame["params"]["session_id"] == "s-unlisted")
        .count();
    println!(
        "TRACE listed_only resumes={} unlisted_resumes={unlisted} cards={}",
        calls.len(),
        far.cards().len()
    );
    assert_eq!(unlisted, 0, "an unlisted session was resumed");
    assert_eq!(far.cards().len(), 1);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn review30_an_accepted_answer_never_survives_a_lost_link_epoch() {
    use serde_json::json;
    use std::sync::Arc;
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    far.follow();
    far.followed();
    gateway.play("question-answered-here", Some("clarify.request"));
    let card = far.card();
    let id = card["id"].as_str().unwrap();
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    let answer = json!({"kind":"answer","answers":{key:{"selected":["green"]}}});
    let held = Arc::new(super::super::fake::Hold::default());
    gateway.world().hold_next = Some(("session.resume", held.clone()));
    far.answer(id, &answer);
    until("answer visit held before its write", || {
        held.entered.load(Ordering::Acquire)
    });
    let (link, _) = far.inner().route.link().unwrap();
    unsafe {
        libc::kill(link.master_pid().unwrap() as libc::pid_t, libc::SIGKILL);
    }
    until("same card returned marked", || {
        far.cards().len() == 1 && far.cards().iter().all(marked)
    });
    held.released.store(true, Ordering::Release);
    let failed = far.delivery(id);
    assert_eq!(failed["state"], "failed");
    until("link returns and reconciles", || {
        far.cards().len() == 1 && !far.cards().iter().any(marked)
    });
    assert!(
        gateway.world().calls("clarify.respond").is_empty(),
        "old answer crossed epochs"
    );
    far.answer(id, &answer);
    until("fresh answer landed", || {
        gateway.world().calls("clarify.respond").len() == 1
    });
    println!(
        "TRACE lost answer epoch old_answer_frames=0 fresh_answer_frames=1 first_receipt=failed"
    );
}
