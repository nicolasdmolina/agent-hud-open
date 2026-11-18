//! Sounds for a far machine (ticket 33), in the lab of `machines.rs`: the
//! machines the app runs, a far gateway behind a throwaway `sshd`, and a
//! real core with sound on that reads the mirrors, so an alert here is
//! one a snapshot carries. The gateway gives no hint: only the polls
//! find a question, and only their looks find a turn's end.
//!
//! A turn of the collector is what `Fleet::snapshot` does first: the
//! turns the far followers told go to core as hook events.
use super::{
    super::{
        fake::{Fake, Hold},
        marked,
    },
    fleet, here,
    mirrors::{far, works},
    owners::aliased,
    ssh, until, Fleet, INSTALL,
};
use agent_hud_agents::hermes::AGENT;
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

const LABEL: &str = "Mac mini";

/// One far machine that is followed, with core reading its mirrors.
fn followed() -> Option<(Fleet, Fake, String)> {
    let mut fleet = fleet(1)?;
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    fleet.write(&[here(), ssh("mini", LABEL, "m0")]);
    fleet.start();
    let key = fleet.connected(LABEL);
    Some((fleet, gateway, key))
}

/// Waits until the gateway has seen `more` polls begin from now.
fn polled(gateway: &Fake, more: usize) {
    let from = gateway.world().listed.len();
    until("the polls go on", || {
        gateway.world().listed.len() >= from + more
    });
}

impl Fleet {
    /// Every alert a snapshot carries, as its kind, session and identity.
    fn alerts(&mut self) -> Vec<(String, String, String)> {
        let snapshot = self.snapshot();
        let items = snapshot["soundAlerts"]["items"].as_array().unwrap();
        let text = |item: &Value, key: &str| item[key].as_str().unwrap().to_owned();
        items
            .iter()
            .map(|item| {
                (
                    text(item, "kind"),
                    text(item, "sessionId"),
                    text(item, "id"),
                )
            })
            .collect()
    }

    /// The session `id` as a snapshot lists it.
    fn session(&mut self, id: &str) -> Option<Value> {
        let snapshot = self.snapshot();
        let sessions = snapshot["sessions"].as_array().unwrap();
        sessions.iter().find(|row| row["id"] == id).cloned()
    }

    /// The session `id` as the list of every session has it, however long
    /// ago it was last written to.
    fn stored(&mut self, id: &str) -> Option<Value> {
        self.turn();
        let query = json!({"scope": "all", "search": ""});
        let found = self.core.request("sessions", query).unwrap();
        let items = found["items"].as_array().unwrap();
        items.iter().find(|row| row["id"] == id).cloned()
    }

    /// Says each alert there is, for the log of a run.
    fn say(&mut self, test: &str) {
        for (kind, session, id) in self.alerts() {
            println!("ALERT {test} kind={kind} session={session} id={id}");
        }
    }

    /// Ends the master of the far machine's link and keeps every new one
    /// from opening: the link is cut. Returns what puts it back.
    fn cut_link(&mut self) -> (std::path::PathBuf, String) {
        let config = self.settings.config.clone().expect("the lab's config");
        let text = std::fs::read_to_string(&config).unwrap();
        let port = format!("Port {}", self.hosts[0].lab.port);
        let closed = format!("Port {}", crate::ssh_link::unix::tests::lab::closed_port());
        std::fs::write(&config, text.replace(&port, &closed)).unwrap();
        for line in self.masters() {
            let pid: i32 = line.split_whitespace().next().unwrap().parse().unwrap();
            // SAFETY: a signal to a master this test's machines started.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        self.stands(LABEL, "cannot-reach");
        (config, text)
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn far_turn_end_sounds_once() {
    let Some((mut fleet, gateway, key)) = followed() else {
        return;
    };
    let id = far(&key, "far-1");
    works(&gateway, "far-1", "Plan the week");
    // Its row is there, and says which machine it runs on.
    assert_eq!(fleet.listed(&id).machine, LABEL);
    polled(&gateway, 2);

    // No step or tool call of the turn makes a sound.
    for kind in [
        "tool.start",
        "tool.complete",
        "message.delta",
        "thinking.delta",
    ] {
        gateway.tells_of_turn("default", "far-1", kind, 1);
        polled(&gateway, 2);
    }
    assert_eq!(fleet.sounds(), (0, 0));
    assert!(fleet.facts.is_empty(), "{:?}", fleet.facts);

    // The turn ends, and nothing says so but the gateway's own stream.
    let before = crate::hooks::now_ms();
    gateway.tells_of_turn("default", "far-1", "message.complete", 1);
    until("the turn's end sounds", || fleet.sounds().1 == 1);
    let after = crate::hooks::now_ms();
    let turn = format!("finished:{}:{id}:epoch-1:1", AGENT.id);
    assert_eq!(fleet.alerts(), [("finished".to_owned(), id.clone(), turn)]);
    fleet.say("far_turn_end_sounds_once");

    // The alert names the far session and no machine: the row does.
    let snapshot = fleet.snapshot();
    let alert = &snapshot["soundAlerts"]["items"][0];
    assert_eq!(alert["source"], AGENT.id);
    assert!(alert.get("machine").is_none(), "{alert}");
    let row = fleet.session(&id).expect("its row");
    assert_eq!(row["machine"], LABEL);
    assert_eq!(row["liveFacts"]["outcome"], "completed");

    // The time on the island's event is when the poll that read the end
    // came back: when it was seen.
    let told: Vec<_> = (fleet.facts.iter())
        .filter(|event| event.kind == "completion")
        .collect();
    assert_eq!(told.len(), 1, "{:?}", fleet.facts);
    assert_eq!(told[0].session_id, id);
    // The fact names the machine, for a row made from it alone.
    assert_eq!(told[0].data["machine"], LABEL);
    let seen = told[0].at_ms;
    assert!((before..=after).contains(&seen), "{before} {seen} {after}");
    let island = snapshot["islandEvents"]["items"].as_array().unwrap();
    let completions: Vec<&Value> = (island.iter())
        .filter(|event| event["kind"] == "completion")
        .collect();
    assert_eq!(completions.len(), 1, "{island:?}");
    assert_eq!(completions[0]["sessionId"], id.as_str());
    let at = chrono::DateTime::parse_from_rfc3339(completions[0]["at"].as_str().unwrap());
    assert_eq!(at.unwrap().timestamp_millis(), seen);
    // No start time is claimed for it.
    assert!(completions[0]["startedAt"].is_null(), "{}", completions[0]);

    // Read again at every poll since, it sounds no second time.
    polled(&gateway, 20);
    assert_eq!(fleet.sounds(), (0, 1));

    // The next turn of the session sounds for itself, also where it came
    // and went between two polls.
    gateway.tells_of_turn("default", "far-1", "message.start", 2);
    gateway.tells_of_turn("default", "far-1", "message.complete", 2);
    until("the next turn's end sounds", || fleet.sounds().1 == 2);
    polled(&gateway, 10);
    assert_eq!(fleet.sounds(), (0, 2));
    fleet.say("far_turn_end_sounds_once");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn far_question_sounds_once() {
    let Some((mut fleet, gateway, key)) = followed() else {
        return;
    };
    fleet.snapshot();
    gateway.asks_quietly("default", "far-q", "p-1");
    let card = fleet.card();
    assert_eq!(card["machine"], LABEL);
    let id = far(&key, "far-q");
    assert_eq!(card["sessionId"], id.as_str());
    until("the question sounds", || fleet.sounds().0 == 1);
    let alerts = fleet.alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!((alerts[0].0.as_str(), &alerts[0].1), ("needs-you", &id));
    assert!(alerts[0].2.ends_with(":p-1"), "{alerts:?}");
    fleet.say("far_question_sounds_once");
    // It waits on, read at every poll, and sounds no second time.
    polled(&gateway, 30);
    assert_eq!(fleet.sounds(), (1, 0));

    // Settled on the far side, and the turn goes on to its end.
    let settled = json!({"prompt_id": "p-1", "execution_generation": 1});
    gateway.tells_quietly("default", "far-q", "clarify.settled", settled);
    until("the card goes", || fleet.cards().is_empty());
    assert_eq!(fleet.sounds(), (1, 0));
    gateway.tells_of_turn("default", "far-q", "message.complete", 1);
    until("the turn's end sounds", || fleet.sounds().1 == 1);
    polled(&gateway, 10);
    assert_eq!(fleet.sounds(), (1, 1));
    fleet.say("far_question_sounds_once");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn no_finished_after_an_outage() {
    let Some((mut fleet, gateway, key)) = followed() else {
        return;
    };
    let id = far(&key, "far-1");
    works(&gateway, "far-1", "Plan the week");
    fleet.listed(&id);
    polled(&gateway, 3);
    let running = fleet.session(&id).expect("its row");
    assert_eq!(running["state"], "running");

    // The link is cut, the turn ends, and the link returns.
    let (config, text) = fleet.cut_link();
    let asked = gateway.world().listed.len();
    gateway.tells_of_turn("default", "far-1", "message.complete", 1);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(gateway.world().listed.len(), asked, "nothing was read");
    std::fs::write(&config, text).unwrap();
    assert_eq!(fleet.connected(LABEL), key);
    polled(&gateway, 20);

    // The end was in the stream when it was seen again: no sound.
    assert_eq!(fleet.sounds(), (0, 0));
    assert!(fleet.facts.is_empty(), "{:?}", fleet.facts);
    assert!(!fleet.log().contains("turn ended"), "{}", fleet.log());
    // The row shows the turn over, as the live list says it now is. No
    // completion was made up for it, so it is not among the recent
    // sessions for having just ended: it is where its stored row puts it.
    until("the row is idle", || {
        fleet.stored(&id).is_some_and(|row| row["state"] == "idle")
    });
    let row = fleet.stored(&id).unwrap();
    assert_eq!(row["machine"], LABEL);
    assert_ne!(row["liveFacts"]["outcome"], "completed");
    assert!(fleet.session(&id).is_none());

    // Seen again on this connection, the next turn's end sounds.
    gateway.tells_of_turn("default", "far-1", "message.start", 2);
    gateway.tells_of_turn("default", "far-1", "message.complete", 2);
    until("the next turn's end sounds", || fleet.sounds().1 == 1);
    fleet.say("no_finished_after_an_outage");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn no_finished_after_a_relaunch() {
    let Some((mut fleet, gateway, key)) = followed() else {
        return;
    };
    let id = far(&key, "far-1");
    works(&gateway, "far-1", "Plan the week");
    fleet.listed(&id);
    polled(&gateway, 3);

    // The app is closed, and the turn ends while it is.
    fleet.stop();
    gateway.tells_of_turn("default", "far-1", "message.complete", 1);
    // Relaunched: a new core on the same data folder, and the machines
    // followed anew.
    fleet.reads_mirrors();
    fleet.start();
    assert_eq!(fleet.connected(LABEL), key);
    polled(&gateway, 20);
    assert_eq!(fleet.sounds(), (0, 0));
    assert!(fleet.facts.is_empty(), "{:?}", fleet.facts);
    // Its row is there from the mirror, under its machine's label.
    assert_eq!(fleet.listed(&id).machine, LABEL);

    // A turn that ends while the app runs sounds.
    gateway.tells_of_turn("default", "far-1", "message.start", 2);
    gateway.tells_of_turn("default", "far-1", "message.complete", 2);
    until("the next turn's end sounds", || fleet.sounds().1 == 1);
    fleet.say("no_finished_after_a_relaunch");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn relaunch_with_a_far_question_waiting() {
    let Some((mut fleet, gateway, key)) = followed() else {
        return;
    };
    fleet.snapshot();
    gateway.asks_quietly("default", "far-q", "p-1");
    fleet.card();
    until("the question sounds", || fleet.sounds().0 == 1);
    fleet.say("relaunch_with_a_far_question_waiting before");

    // The app is closed with the question waiting. Another is asked
    // while it is closed.
    fleet.stop();
    gateway.asks_quietly("default", "far-other", "p-2");
    // Relaunched. Core's first snapshot comes at once and finds nothing
    // waiting: the far machine is not followed for twenty seconds more.
    fleet.reads_mirrors();
    assert_eq!(fleet.sounds(), (0, 0));
    let began = std::time::Instant::now();
    while began.elapsed() < Duration::from_secs(20) {
        assert_eq!(fleet.sounds(), (0, 0));
        assert!(fleet.cards().is_empty());
        std::thread::sleep(Duration::from_millis(250));
    }
    fleet.start();
    until("both cards arrive", || fleet.cards().len() == 2);
    // The one decided before the relaunch does not sound again. The one
    // asked while the app was closed sounds, once.
    until("the new question sounds", || fleet.sounds().0 == 1);
    polled(&gateway, 20);
    let alerts = fleet.alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].0, "needs-you");
    assert_eq!(alerts[0].1, far(&key, "far-other"));
    assert!(alerts[0].2.ends_with(":p-2"), "{alerts:?}");
    fleet.say("relaunch_with_a_far_question_waiting after");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn handover_sounds_nothing() {
    let Some((mut fleet, gateway, key)) = aliased() else {
        return;
    };
    fleet.snapshot();
    gateway.asks_quietly("default", "s-asks", "p-asks");
    works(&gateway, "s-works", "A turn under way");
    let card = fleet.card();
    until("the question sounds", || fleet.sounds().0 == 1);
    polled(&gateway, 3);
    let sounds = fleet.sounds();
    assert_eq!(sounds, (1, 0));
    let number = fleet.number();

    // The owner's entry is removed, and the handover is held where the
    // alias says who it is again: the route number has gone up, every
    // follower has lost its socket, and no route is behind the machine.
    let held = Arc::new(Hold::default());
    gateway.world().hold_next = Some(("profiles.list", held.clone()));
    fleet.write(&[here(), ssh("b", "Mini over VPN", "m0b")]);
    until("the alias is asked who it is", || {
        held.entered.load(Ordering::Acquire)
    });
    until("the card is marked", || fleet.cards().iter().all(marked));
    assert!(fleet.number() > number, "the route changed");
    assert_eq!(fleet.sounds(), sounds);
    // A turn of another session ends during the handover, on no route.
    // (Ended a moment before the list was written, with the old route
    // still connected, it would rightly sound: the session had been
    // seen, and the HUD was connected when it ended.)
    gateway.tells_of_turn("default", "s-works", "message.complete", 1);
    held.released.store(true, Ordering::Release);

    // The alias takes the machine over, and the card is the same card.
    assert_eq!(fleet.connected("Mini over VPN"), key, "the same machine");
    let after = fleet.reached();
    assert_eq!(after["id"], card["id"]);
    assert_eq!(after["machine"], "Mini over VPN");
    polled(&gateway, 20);
    // No alert of either kind: the question is not asked again, and the
    // end that came during the handover was there when the new route
    // first saw the session.
    assert_eq!(fleet.sounds(), sounds);
    assert!(fleet.facts.is_empty(), "{:?}", fleet.facts);
    // Seen again over the new route, a turn's end sounds as before, and
    // the fact says the label the machine goes by now.
    gateway.tells_of_turn("default", "s-works", "message.start", 2);
    gateway.tells_of_turn("default", "s-works", "message.complete", 2);
    until("a turn's end sounds", || fleet.sounds().1 == 1);
    assert!(!fleet.facts.is_empty());
    for fact in &fleet.facts {
        assert_eq!(fact.data["machine"], "Mini over VPN", "{fact:?}");
    }
    fleet.say("handover_sounds_nothing");
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn local_turn_end_still_sounds_once() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    // This machine's own gateway, with a session in a turn.
    let local = Fake::start(&fleet.local, "instance-local");
    fleet.write(&[here()]);
    fleet.start();
    fleet.snapshot();
    local.tells_of_turn("default", "local-1", "message.start", 1);
    polled(&local, 3);
    local.tells_of_turn("default", "local-1", "message.complete", 1);
    polled(&local, 20);
    // Its follower read the session while it worked, and looks at none
    // that is idle: it tells no turn, and asks for no stream but that of
    // a session at work.
    assert!(fleet.heard.lock().unwrap().is_empty());
    assert!(!fleet.log().contains("turn "), "{}", fleet.log());
    assert_eq!(fleet.sounds(), (0, 0));
    let looked = local.world().calls("session.events.since").len();
    polled(&local, 20);
    assert_eq!(local.world().calls("session.events.since").len(), looked);

    // Its hook says the turn ended, as Hermes runs it on this machine.
    let payload = json!({"hook_event_name": "on_session_end", "session_id": "local-1",
        "cwd": "/synthetic/project", "extra": {"turn_id": "local-1:task-1:turn"}});
    let now = crate::hooks::now_ms();
    let event = agent_hud_agents::parse_event(AGENT.id, "--completion-hook", &payload, now);
    fleet.core.ingest_hook_events(vec![event.unwrap().unwrap()]);
    until("the turn's end sounds", || fleet.sounds().1 == 1);
    polled(&local, 20);
    assert_eq!(fleet.sounds(), (0, 1));
    let alerts = fleet.alerts();
    assert_eq!(alerts[0].1, format!("{}:local-1", AGENT.id));
    assert!(fleet.facts.is_empty());
    fleet.say("local_turn_end_still_sounds_once");
}
