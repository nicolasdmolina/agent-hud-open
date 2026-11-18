//! A poll that comes back late (ticket 58). Its replies were read over a
//! link that was then lost, and it is handed to the model only after an
//! answer that could not be sent had marked the card as out of reach.
//! That is the ordering ticket 56 found under load, where the late poll
//! showed the card as reachable on the dead link. Each poll carries the
//! link it was read over, and one whose link was lost since, or gave way
//! to another, is dropped: an older reading never undoes a newer mark.
//!
//! The poll is held where ticket 56 put its delay: after its last reply,
//! before the model is locked for it (`tap.rs`, `Late`).
use super::{
    super::{fake::Fake, marked},
    far_lab, until, FarLab,
};
use serde_json::{json, Value};

/// A far machine with one question showing, and the next poll held once
/// it has read everything: the card, and the link the poll was read on.
fn a_poll_held_after_its_replies() -> Option<(FarLab, Fake, Value, u64)> {
    let mut far = far_lab()?;
    let gateway = far.gateway("instance-1");
    far.follow();
    far.followed();
    gateway.play("question-answered-here", Some("clarify.request"));
    let card = far.card();
    assert!(!marked(&card));
    let follower = far.inner().followers().unwrap().set()[0].clone();
    // A poll that begins from here on: it reads the session whole and
    // finds the request waiting, which is what takes a mark away.
    let listed = gateway.world().listed.len();
    until("a poll began", || gateway.world().listed.len() > listed);
    follower.late.arm();
    until("a poll has read its replies", || follower.late.held());
    let (_, number) = far.inner().route.link().expect("the link it was read on");
    Some((far, gateway, card, number))
}

/// Ends the link's master with no word to anyone, then answers the card:
/// the answer finds no way and the card comes back marked, while the
/// poll is still held. The answer's receipt says it failed.
fn lose_the_link_and_answer(far: &FarLab, card: &Value) {
    let (link, _) = far.inner().route.link().expect("a link");
    let pid = link.master_pid().expect("its master");
    // SAFETY: a signal to the master this test's observer started.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    let id = card["id"].as_str().unwrap();
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    let answer = json!({"kind": "answer", "answers": {key: {"selected": ["green"]}}});
    far.answer(id, &answer);
    assert_eq!(far.delivery(id)["state"], "failed");
    until("the card is back, marked", || {
        far.cards().len() == 1 && far.cards().iter().all(marked)
    });
    assert!(far.log().contains(" undelivered id="), "{}", far.log());
}

/// What the log says from the answer that could not be sent on.
fn since_undelivered(far: &FarLab) -> String {
    let log = far.log();
    let from = log.find(" undelivered id=").expect("the answer failed");
    log[from..].to_owned()
}

fn lost(far: &FarLab) -> usize {
    far.log().matches("connection failed reason=").count()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_poll_read_before_its_link_was_lost_never_clears_the_mark() {
    let Some((far, gateway, card, _)) = a_poll_held_after_its_replies() else {
        return;
    };
    // No link opens again: the dead one stays the last there was.
    let config = far.lab.config_text();
    let closed = crate::ssh_link::unix::tests::lab::closed_port();
    let (open, shut) = (format!("Port {}", far.lab.port), format!("Port {closed}"));
    std::fs::write(far.lab.at("config"), config.replace(&open, &shut)).unwrap();
    lose_the_link_and_answer(&far, &card);
    let before = lost(&far);
    let follower = far.inner().followers().unwrap().set()[0].clone();
    follower.late.free();
    // The poll came back and its follower went on: the socket is lost.
    until("the follower loses its socket", || lost(&far) > before);
    let said = since_undelivered(&far);
    assert!(
        !said.contains(" reached id="),
        "a poll read before the link was lost showed the card as reachable:\n{said}"
    );
    // And it stays so once the keeper has said the link is lost.
    until("no link stands", || far.inner().route.link().is_none());
    assert_eq!(far.cards().len(), 1);
    assert!(far.cards().iter().all(marked), "marked on the dead link");
    assert!(gateway.world().calls("clarify.respond").is_empty());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_poll_read_on_the_link_before_never_clears_the_mark_on_the_next() {
    let Some((far, gateway, card, read_on)) = a_poll_held_after_its_replies() else {
        return;
    };
    lose_the_link_and_answer(&far, &card);
    // The keeper opens the next link while the poll is still held.
    until("the next link is up", || {
        let link = far.inner().route.link();
        link.is_some_and(|(_, number)| number > read_on)
    });
    assert!(far.cards().iter().all(marked), "still marked");
    let before = lost(&far);
    let follower = far.inner().followers().unwrap().set()[0].clone();
    follower.late.free();
    until("the follower loses its socket", || lost(&far) > before);
    // Up to the loss of its socket, nothing said the card was reachable:
    // the poll of the link before was dropped.
    let said = since_undelivered(&far);
    let until_lost = said.split("connection failed reason=").next().unwrap();
    assert!(
        !until_lost.contains(" reached id="),
        "a poll read on the link before showed the card as reachable:\n{said}"
    );
    // A poll read on the new link is what takes the mark away, and the
    // card is answered there.
    until("a poll on the new link reaches the card", || {
        far.cards().len() == 1 && !far.cards().iter().any(marked)
    });
    let after = since_undelivered(&far);
    let ready = after.find("connection ready").expect("connected again");
    let reached = after.find(" reached id=").expect("reached again");
    assert!(
        ready < reached,
        "reached only once connected again:\n{after}"
    );
    assert!(gateway.world().calls("clarify.respond").is_empty());
    let id = card["id"].as_str().unwrap();
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    let answer = json!({"kind": "answer", "answers": {key: {"selected": ["green"]}}});
    far.answer(id, &answer);
    until("the fresh answer landed", || {
        gateway.world().calls("clarify.respond").len() == 1
    });
}
