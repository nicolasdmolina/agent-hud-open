//! A card whose gateway cannot be reached (ticket 27). It stays, says so,
//! and takes no answer; its time, its hold and whether it was dismissed
//! are untouched; the first poll after the connection is back takes the
//! mark away or closes the card as Hermes says. Nothing here adds a timer
//! or takes one away: the hold still ends a marked card at its deadline.
//!
//! An outage is the pretend gateway stopped (`Fake::stop`: every socket
//! cut, its control socket gone), and its return is the same instance
//! started again in the same home with the recording played into it. Time
//! that passes is core's own clock, which every call on core's requests is
//! given: a hold of minutes is run through without waiting for it.
//! The answers that lose a race, and the failures that come late, are in
//! `unreachable_late.rs`.
use super::super::Shared;
use super::{fake::Fake, lab, marked, plain, until, Lab};
use agent_hud_agents::hermes::{gateway::out_of_reach, AGENT};
use agent_hud_core::AgentHudCore;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

/// The recording of one question, and the answer that picks its first
/// choice.
pub const ASKED: &str = "question-answered-elsewhere";
pub fn red() -> Value {
    json!({"kind": "answer", "answers": {"e1bc9d4fa6": {"selected": ["red"]}}})
}
pub const MINUTE: i64 = 60_000;

/// A core with sound on, whose requests the lab's connection registers
/// its cards with: its snapshots say which alerts sounded.
pub fn sounding(lab: &mut Lab) -> AgentHudCore {
    let mut core = AgentHudCore::new(vec![], lab.data.join("core"));
    let sound = json!({"key": "sound", "value": true});
    core.request("set_setting", sound).unwrap();
    lab.requests = core.requests();
    core
}

pub fn snapshot(core: &mut AgentHudCore) -> Value {
    core.request("snapshot", json!({})).unwrap()
}

/// How many times `needs-you` sounded so far.
pub fn needs(core: &mut AgentHudCore) -> usize {
    let snapshot = snapshot(core);
    let items = snapshot["soundAlerts"]["items"].as_array().unwrap();
    let needs = items.iter().filter(|item| item["kind"] == "needs-you");
    needs.count()
}

/// The gateway, followed, with its first polls behind it.
pub fn started(lab: &mut Lab) -> Fake {
    started_at(lab, super::quick())
}

/// The same, followed at `pace`.
pub fn started_at(lab: &mut Lab, pace: super::super::Pace) -> Fake {
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow_at(pace);
    until("the first polls ran", || {
        gateway.world().calls("session.active_list").len() >= 2
    });
    gateway
}

/// Hermes asks the recorded question; the id of its card.
pub fn asks(lab: &Lab, gateway: &Fake) -> String {
    lab.hears(gateway, ASKED, "clarify.request");
    lab.card()["id"].as_str().unwrap().to_owned()
}

pub fn follower(lab: &Lab) -> Arc<Shared> {
    lab.observer.as_ref().unwrap().local()
}

/// How often the follower registered a card with core.
pub fn shows(lab: &Lab) -> usize {
    follower(lab).shows.load(Ordering::Acquire)
}

/// When the request came and when its hold ends, as core holds them.
pub fn times(lab: &Lab, card: &Value) -> (i64, i64) {
    let (id, session) = (card["id"].as_str().unwrap(), card["sessionId"].as_str());
    let came = lab.requests.waiting_at(AGENT.id, session.unwrap());
    (came.unwrap(), lab.requests.deadline(id).unwrap())
}

/// The gateway stops, and the card says it cannot be reached.
pub fn outage(lab: &Lab, gateway: &mut Fake) {
    gateway.stop();
    until("the card says its gateway cannot be reached", || {
        lab.cards().first().is_some_and(marked)
    });
}

/// The same instance is back in the same home, with `through` of the
/// recording played into it: the question still waiting, or settled.
pub fn returns(lab: &Lab, through: &str) -> Fake {
    returns_with(lab, |gateway| {
        lab.hears(gateway, ASKED, through);
    })
}

/// The same instance is back in the same home, holding what `seed` puts
/// into it before its follower can poll it. The follower's model is held
/// meanwhile: it takes no answer to who is there, so it begins no poll,
/// until the gateway holds its session again. Without that a poll could
/// find the gateway empty, close the card as a session that ended, and
/// open it anew once the recording was in (the first review of ticket
/// 27, finding 3).
pub fn returns_with(lab: &Lab, seed: impl FnOnce(&Fake)) -> Fake {
    let shared = follower(lab);
    let _model = shared.model();
    let gateway = Fake::start(&lab.home, "instance-1");
    seed(&gateway);
    gateway
}

/// Waits until a poll of the gateway that is back has read the session.
pub fn read_again(gateway: &Fake) {
    until("the gateway that is back is polled", || {
        gateway.world().calls("session.events.since").len() >= 2
    });
}

/// Whether the same request would be refused if it were registered again:
/// core counts it as dismissed. Where it is not, it is taken away again.
pub fn dismissed(lab: &Lab, card: &Value) -> bool {
    let id = card["id"].as_str().unwrap();
    let now = crate::hooks::now_ms();
    let registered = lab.requests.register_native(id.into(), card.clone(), now);
    if registered.is_some() {
        lab.requests.cancel(id);
    }
    registered.is_none()
}

#[test]
fn a_card_outlives_an_outage_shorter_than_its_hold() {
    let mut lab = lab();
    lab.requests.set_hold_ms(30 * MINUTE as u64);
    let mut gateway = started(&mut lab);
    asks(&lab, &gateway);
    let card = lab.card();
    let (came, deadline) = times(&lab, &card);
    assert_eq!(deadline - came, 30 * MINUTE);

    outage(&lab, &mut gateway);
    // Twelve minutes with the gateway stopped: the card is there
    // throughout, marked, and it is the card that was there before.
    for minute in 0..=12 {
        let cards = lab.requests.views(came + minute * MINUTE);
        assert_eq!(cards.len(), 1, "minute {minute}");
        assert!(marked(&cards[0]), "minute {minute}");
        assert_eq!(cards[0]["unreachable"], json!(out_of_reach()));
        assert_eq!(plain(cards[0].clone()), card, "minute {minute}");
        assert_eq!(times(&lab, &card), (came, deadline));
    }
    // Nothing closed it, its follower lives and goes on asking, and the
    // session goes on waiting.
    assert!(!lab.log().contains("close id="), "{}", lab.log());
    assert!(follower(&lab).active());
    assert_eq!(lab.waiting(), [card["sessionId"].as_str().unwrap()]);
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

/// A card under the default hold whose gateway stopped: marked, and gone
/// at its original deadline. Returns what the later test goes on from.
fn expired(lab: &mut Lab, core: &mut AgentHudCore) -> (Value, usize) {
    let mut gateway = started(lab);
    snapshot(core);
    asks(lab, &gateway);
    assert_eq!(needs(core), 1, "the question sounds once");
    let card = lab.card();
    let id = card["id"].as_str().unwrap();
    let (came, deadline) = times(lab, &card);
    assert_eq!(deadline - came, 10 * MINUTE, "the default hold");
    let registered = shows(lab);

    outage(lab, &mut gateway);
    let sockets = gateway.world().sockets;
    // One moment before its deadline it is there, marked. At the
    // deadline it goes, as a card that can be reached does.
    let last = lab.requests.views(deadline - 1);
    assert!(last.len() == 1 && marked(&last[0]), "{last:?}");
    assert!(lab.requests.views(deadline).is_empty());
    assert_eq!(lab.requests.deadline(id), None);
    // Nothing was sent for it, and it counts as dismissed.
    let world = gateway.world();
    assert!(world.calls("clarify.respond").is_empty());
    assert_eq!(world.sockets, sockets);
    drop(world);
    assert!(dismissed(lab, &card));
    assert!(lab.requests.deliveries(deadline).is_empty());
    assert_eq!(needs(core), 1, "no sound for a card that expired");
    (card, registered)
}

#[test]
fn the_hold_still_ends_a_marked_card() {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    expired(&mut lab, &mut core);
}

#[test]
fn an_expired_card_does_not_come_back() {
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let (card, registered) = expired(&mut lab, &mut core);
    let id = card["id"].as_str().unwrap();

    // The gateway returns with the question still waiting.
    let gateway = returns(&lab, "clarify.request");
    read_again(&gateway);
    let shared = follower(&lab);
    until("the poll finds the request waiting", || {
        !shared.model().gateway.marked(id)
    });
    assert!(shared.model().gateway.pending(id).is_some());
    read_again(&gateway);
    // No card, and no sound.
    assert!(lab.cards().is_empty());
    assert_eq!(needs(&mut core), 1);
    assert_eq!(shows(&lab), registered, "it was not registered again");
    assert!(dismissed(&lab, &card));
    assert_eq!(lab.log().matches("open id=").count(), 1, "{}", lab.log());
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

#[test]
fn a_mark_does_not_move_the_deadline() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    asks(&lab, &gateway);
    let card = lab.card();
    let before = times(&lab, &card);

    outage(&lab, &mut gateway);
    let during = lab.card();
    assert!(marked(&during));
    assert_eq!(during["createdAt"], card["createdAt"]);
    assert_eq!(times(&lab, &card), before, "while marked");

    let _gateway = returns(&lab, "clarify.request");
    until("the mark clears", || !marked(&lab.card()));
    assert_eq!(lab.card(), card, "the card as it was");
    assert_eq!(times(&lab, &card), before, "after the mark cleared");
    // Marked and unmarked in place, as the one request it always was.
    let log = lab.log();
    assert_eq!(log.matches("open id=").count(), 1, "{log}");
    assert_eq!(log.matches("unreachable id=").count(), 1, "{log}");
    assert_eq!(log.matches("reached id=").count(), 1, "{log}");
    assert_eq!(shows(&lab), 1);
}

/// The gate. The gateway itself stays up here and would take an answer:
/// only its follower has lost it, and cannot find it again. So what keeps
/// an answer from it is the rule, not a gateway that is not there.
#[test]
fn a_marked_card_takes_no_answer() {
    let mut lab = lab();
    let gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let card = lab.card();
    let before = times(&lab, &card);
    // Nobody says who is there any more, and the socket drops.
    let told = gateway.world().identify.take();
    gateway.cut();
    until("the card is marked", || marked(&lab.card()));
    // The follower has asked again and found nobody who says who is
    // there: everything the loss itself told core is told by now.
    until("the follower finds no way back", || {
        lab.log().contains("connection failed reason=Invalid")
    });
    let (sockets, tickets) = {
        let world = gateway.world();
        (world.sockets, world.granted.len())
    };
    let revision = lab.requests.revision();

    let now = crate::hooks::now_ms();
    for decision in [red(), json!({"kind": "allow"})] {
        let refused = lab.requests.answer(&id, &decision, now);
        assert_eq!(refused, Err(out_of_reach()), "{decision}");
    }
    // Long enough for the answers' thread to have looked many times.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let world = gateway.world();
    assert_eq!(world.sockets, sockets, "no socket was opened");
    assert_eq!(world.granted.len(), tickets, "no ticket was asked for");
    assert!(world.calls("clarify.respond").is_empty());
    assert!(world.calls("session.resume").is_empty());
    drop(world);
    // Not removed, not dismissed, and its hold is where it was.
    let still = lab.card();
    assert!(marked(&still));
    assert_eq!(plain(still), card);
    assert_eq!(times(&lab, &card), before);
    assert_eq!(lab.requests.revision(), revision, "nothing changed in core");
    assert!(lab.requests.deliveries(now).is_empty());
    assert!(!lab.log().contains("undelivered"), "{}", lab.log());

    // Found again, the same card takes the same answer.
    gateway.world().identify = told;
    until("the mark clears", || !marked(&lab.card()));
    lab.requests.answer(&id, &red(), now).unwrap();
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    assert_eq!(gateway.world().calls("clarify.respond").len(), 1);
}

#[test]
fn same_instance_returns() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    let card = lab.card();
    outage(&lab, &mut gateway);

    let gateway = returns(&lab, "clarify.request");
    until("the mark clears", || !marked(&lab.card()));
    assert_eq!(lab.card(), card);
    assert!(!lab.log().contains("close id="), "{}", lab.log());
    // The card is answered, and Hermes goes on with the turn.
    lab.requests
        .answer(&id, &red(), crate::hooks::now_ms())
        .unwrap();
    assert_eq!(lab.delivery(&id)["state"], "confirmed");
    until("the card goes", || lab.cards().is_empty());
    let world = gateway.world();
    let answers = world.calls("clarify.respond");
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].1["params"]["answer"], "red");
    assert_eq!(answers[0].1["params"]["prompt_id"], "e1bc9d4fa6");
    drop(world);
    assert!(lab.log().contains("why=Answered"), "{}", lab.log());
    until("the session stops waiting", || lab.waiting().is_empty());
    // One outage and return, as the connection logged it.
    eprintln!("--- outage and return ---\n{}--- end ---", lab.log());
}

#[test]
fn a_new_instance_closes_the_card() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    outage(&lab, &mut gateway);
    // Another instance in the same home: what the old one asked did not
    // survive it. Out of reach closed nothing; the restart does.
    let _gateway = Fake::start(&lab.home, "instance-2");
    until("the card goes", || lab.cards().is_empty());
    let closed = format!("close id={id} why=GatewayRestarted");
    assert!(lab.log().contains(&closed), "{}", lab.log());
    assert!(!lab.log().contains("reached id="), "{}", lab.log());
    until("the session stops waiting", || lab.waiting().is_empty());
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
}

/// The lines the log holds from the moment the gateway was ready again.
fn since_ready(lab: &Lab) -> Vec<String> {
    let log = lab.log();
    let lines: Vec<&str> = log.lines().collect();
    let ready = |line: &&str| line.contains("connection ready");
    let from = lines.iter().rposition(ready).unwrap();
    let said = |line: &&str| line.split_once(' ').map(|(_, said)| said.to_owned());
    lines[from..].iter().filter_map(said).collect()
}

#[test]
fn answered_elsewhere_during_the_outage() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    let id = asks(&lab, &gateway);
    outage(&lab, &mut gateway);
    // While it was out of reach, the session's own client answered.
    let _gateway = returns(&lab, "clarify.settled");
    until("the card goes", || lab.cards().is_empty());
    // Closed by the first poll: nothing is said of the card between the
    // gateway being ready and its close.
    let after = since_ready(&lab);
    assert_eq!(after[1], format!("close id={id} why=Settled"), "{after:?}");
    assert!(!lab.log().contains("reached id="), "{}", lab.log());
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
}

#[test]
fn hermes_timed_out_during_the_outage() {
    let mut lab = lab();
    let mut gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-timeout", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    outage(&lab, &mut gateway);
    // Hermes's own wait ran out meanwhile: the whole recording.
    let _gateway = returns_with(&lab, |gateway| {
        gateway.play("question-timeout", None);
        gateway.say("sessions.changed");
    });
    until("the card goes", || lab.cards().is_empty());
    let after = since_ready(&lab);
    assert!(
        after[1].starts_with(&format!("close id={id} why=")),
        "{after:?}"
    );
    assert!(!lab.log().contains("reached id="), "{}", lab.log());
    // Nobody answered here: no note of a delivery that failed.
    assert!(lab.requests.deliveries(crate::hooks::now_ms()).is_empty());
    assert!(!lab.log().contains("undelivered"), "{}", lab.log());
}

#[test]
fn one_needs_you_across_mark_and_unmark() {
    for dismiss in [false, true] {
        let mut lab = lab();
        let mut core = sounding(&mut lab);
        let mut gateway = started(&mut lab);
        snapshot(&mut core);
        let id = asks(&lab, &gateway);
        let card = lab.card();
        assert_eq!(needs(&mut core), 1, "the question sounds once");
        if dismiss {
            // Put away before the outage: it is not shown again after it.
            let leave = json!({"kind": "leave"});
            lab.requests
                .answer(&id, &leave, crate::hooks::now_ms())
                .unwrap();
            until("the card is put away", || lab.cards().is_empty());
        }
        let shared = follower(&lab);
        gateway.stop();
        until("the model marks the card", || {
            shared.model().gateway.marked(&id)
        });
        assert_eq!(needs(&mut core), 1, "marked, dismiss={dismiss}");
        assert_eq!(lab.cards().len(), usize::from(!dismiss));

        let gateway = returns(&lab, "clarify.request");
        until("the poll finds the request waiting", || {
            !shared.model().gateway.marked(&id)
        });
        read_again(&gateway);
        assert_eq!(needs(&mut core), 1, "unmarked, dismiss={dismiss}");
        if dismiss {
            assert!(lab.cards().is_empty(), "a dismissed card came back");
            assert!(dismissed(&lab, &card));
        } else {
            assert_eq!(lab.card(), card);
        }
        assert_eq!(shows(&lab), 1, "one request, registered once");
    }
}

#[test]
fn five_failed_reconnects_change_nothing() {
    let mut lab = lab();
    let mut gateway = started(&mut lab);
    asks(&lab, &gateway);
    let card = lab.card();
    let times_before = times(&lab, &card);
    outage(&lab, &mut gateway);
    let shared = follower(&lab);
    // The first reconnect has failed, and said so.
    until("the follower finds nobody there", || {
        lab.log().contains("connection failed reason=Unreachable")
    });
    let (marked_card, revision, log) = (lab.card(), lab.requests.revision(), lab.log());
    // Five more times the follower asks who is there, and nobody is.
    let asked = |lab: &Lab| {
        let verbs = &lab.route.asked().verbs;
        verbs.iter().filter(|verb| *verb == "identify").count()
    };
    let before = asked(&lab);
    until("five more reconnects failed", || asked(&lab) >= before + 5);
    assert_eq!(lab.card(), marked_card);
    assert_eq!(times(&lab, &card), times_before);
    assert_eq!(lab.requests.revision(), revision, "nothing changed in core");
    assert_eq!(lab.log(), log, "nothing more was said");
    assert!(Arc::ptr_eq(&shared, &follower(&lab)) && shared.active());
    assert!(lab.observer.as_ref().unwrap().ends().is_empty());
    assert_eq!(shows(&lab), 1);
}

/// What a snapshot of core says the session does.
fn state(core: &mut AgentHudCore, id: &str) -> Option<String> {
    let snapshot = snapshot(core);
    let rows = snapshot["sessions"].as_array()?;
    let row = rows.iter().find(|row| row["id"] == id)?;
    row["state"].as_str().map(str::to_owned)
}

#[test]
fn a_session_with_a_marked_card_shows_as_waiting_after_its_reading_has_lapsed() {
    use agent_hud_core::requests::LIVE_STATES_MS;
    let mut lab = lab();
    let mut core = sounding(&mut lab);
    let mut gateway = started(&mut lab);
    asks(&lab, &gateway);
    let card = lab.card();
    let session = card["sessionId"].as_str().unwrap().to_owned();
    until("the gateway's reading of the session is told", || {
        let all = lab.requests.native_states();
        all.of(AGENT.id, "local")
            .is_some_and(|read| read.sessions.contains_key(&session))
    });
    assert_eq!(state(&mut core, &session).as_deref(), Some("waiting"));

    outage(&lab, &mut gateway);
    // Its reading is over: gone with the socket, and it would have lapsed
    // by itself half a minute after the last poll.
    lab.requests
        .expire(crate::hooks::now_ms() + LIVE_STATES_MS + 1);
    assert!(lab.requests.native_states().of(AGENT.id, "local").is_none());
    // The session still shows as waiting, on its marked card.
    assert!(marked(&lab.card()));
    assert_eq!(lab.waiting(), [session.as_str()]);
    assert_eq!(state(&mut core, &session).as_deref(), Some("waiting"));
}
