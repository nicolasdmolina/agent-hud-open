//! A gateway that restarts, a socket that drops, and polls that never run
//! two at a time.
use super::{fake::Fake, lab, until};
use std::time::Duration;

#[test]
fn a_gateway_killed_mid_question_comes_back_and_the_cards_follow() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "gateway-restart-hard", "approval.request");
    let before = lab.card();

    // Killed: the socket is cut and the control socket is gone. The
    // request may still wait, so the card stays while nothing answers.
    gateway.kill();
    until("the loss is seen", || {
        lab.log().contains("connection failed")
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(lab.cards().len(), 1);
    assert!(!lab.log().contains("close id="), "{}", lab.log());

    // It comes back as another instance on another port, with nothing
    // waiting: what was asked before did not survive, and its card goes.
    let gateway = Fake::start(&lab.home, "instance-2");
    until("the card of the old instance goes", || {
        lab.cards().is_empty()
    });
    assert!(lab.log().contains("why=GatewayRestarted"), "{}", lab.log());
    until("the session stops waiting", || lab.waiting().is_empty());

    // The same observer goes on: the next request shows and settles.
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let after = lab.card();
    assert_ne!(after["id"], before["id"]);
    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the new card goes", || lab.cards().is_empty());
    // Only the first socket of the new instance was needed.
    assert_eq!(gateway.world().sockets, 1);
    assert_eq!(gateway.world().turned_away, 0);
}

#[test]
fn a_socket_that_drops_keeps_the_card_and_the_connection_comes_back() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    let card = lab.card();

    gateway.cut();
    until("the socket is opened again", || {
        gateway.world().sockets == 2
    });
    until("polls run again", || {
        let lists = gateway.world().calls("session.active_list");
        lists.iter().any(|(socket, _)| *socket == 2)
    });
    // The same instance: the request still waits, under the same card. It
    // said its gateway was out of reach while the socket was down, and is
    // as it was once a poll on the new socket has found the request
    // waiting (ticket 27).
    assert_eq!(lab.cards().len(), 1);
    until("the card is as it was", || lab.cards() == [card.clone()]);
    assert!(!lab.log().contains("close id="), "{}", lab.log());

    gateway.play("question-answered-elsewhere", Some("clarify.settled"));
    until("the card goes", || lab.cards().is_empty());
    assert!(lab.log().contains("why=Settled"), "{}", lab.log());
}

#[test]
fn only_one_poll_of_a_gateway_runs_at_a_time_however_many_ask() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    gateway.world().slow = Duration::from_millis(25);
    lab.follow();
    lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
    lab.card();
    // Hooks and hints ask for a poll over and over while polls are slow.
    let before = gateway.world().calls("session.active_list").len();
    until("several more polls ran", || {
        lab.observer.as_ref().unwrap().nudge();
        gateway.say("platforms.changed");
        std::thread::sleep(Duration::from_millis(5));
        gateway.world().calls("session.active_list").len() > before + 10
    });
    let world = gateway.world();
    assert_eq!(world.lists_at_once, 1);
    // One socket did all of it.
    assert_eq!(world.sockets, 1);
}

#[test]
fn a_hook_or_a_hint_starts_a_poll_without_waiting_for_the_slow_timer() {
    for hook in [true, false] {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        lab.follow();
        // The first poll finds no live session: the next is ten seconds off.
        until("the first poll ran", || {
            gateway.world().calls("session.active_list").len() >= 2
        });
        std::thread::sleep(Duration::from_millis(400));
        gateway.play("question-answered-elsewhere", Some("clarify.request"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(lab.cards().is_empty());
        if hook {
            lab.observer.as_ref().unwrap().nudge();
        } else {
            gateway.say("sessions.changed");
        }
        lab.card();
    }
}
