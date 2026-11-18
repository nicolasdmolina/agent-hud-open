//! Live token counts of a far session (`sample.rs`): which sessions a
//! round of count calls asks `session.stats` of, what a reply is taken
//! for, that no poll asks one, and that a session's last count is asked
//! until it is answered. `poll` here is a poll and the round of count
//! calls that follows it (`fake.rs`).
use super::super::*;
use super::fake::{ask, count, identified, names, poll, Hermes};
use super::{far, fixture, local, INSTALL};
use serde_json::{json, Value};

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

/// What `session.stats` says of a session that used these four counts.
fn stats(input: u64, output: u64, read: u64, write: u64) -> Value {
    json!({"input_tokens": input, "output_tokens": output, "cache_read_tokens": read,
        "cache_write_tokens": write, "total_tokens": input + output, "model": "claude-test"})
}

/// A session of the default profile that the list says does `status`.
fn does(hermes: &mut Hermes, session: &str, status: &str, said: Value) {
    let stream = hermes.stream(&Profile::Default, session);
    stream.status = Some(status.into());
    stream.stats = said;
}

/// The sessions asked `session.stats` since the last poll began, in
/// order.
fn asked(hermes: &Hermes) -> Vec<String> {
    let stats = |step: &Step| match step {
        Step::Stats(session) => Some(session.id().to_owned()),
        _ => None,
    };
    hermes.asked.iter().filter_map(stats).collect()
}

/// The samples the last round took of the default profile, by session.
fn samples(hermes: &Hermes) -> Vec<(String, u64)> {
    let of = hermes.counted.get(&Profile::Default);
    let total = |(id, sample): &(String, Sample)| (id.clone(), sample.total());
    of.map(|of| of.iter().map(total).collect())
        .unwrap_or_default()
}

#[test]
fn a_far_poll_asks_stats_once_for_each_session_that_works() {
    let (mut g, mut h) = start();
    does(&mut h, "local-idle", "idle", stats(900, 90, 0, 0));
    does(&mut h, "local-starts", "starting", stats(10, 1, 0, 0));
    does(&mut h, "local-waits", "waiting", stats(40, 4, 0, 0));
    does(&mut h, "local-works", "working", stats(100, 20, 30, 5));
    poll(&mut g, &mut h, 1_000);
    // One call, and none for a session that is idle, only waits or only
    // starts: its count does not move.
    assert_eq!(asked(&h), ["local-works"]);
    assert_eq!(samples(&h), [("local-works".to_owned(), 155)]);
    assert_eq!(
        h.counted[&Profile::Default][0].1,
        Sample {
            input: 100,
            output: 20,
            cache_read: 30,
            cache_write: 5
        }
    );
    // The same at the next poll, for as long as it works. One that goes
    // on starting, as a session can for days, is never asked.
    for now in [2_000, 3_000, 4_000] {
        poll(&mut g, &mut h, now);
        assert_eq!(asked(&h), ["local-works"]);
    }
}

/// A turn that began and ended between two polls: the session was
/// listed as starting, and is idle at the next poll. It is asked once.
/// So is one that worked and starts again: a turn ended in between.
#[test]
fn a_session_that_started_or_worked_and_is_listed_otherwise_is_asked_once() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "starting", stats(0, 0, 0, 0));
    poll(&mut g, &mut h, 1_000);
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    does(&mut h, "local-turn", "idle", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 3_000);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 120)]);
    poll(&mut g, &mut h, 4_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    // Working, then starting again, and starting still.
    does(&mut h, "local-turn", "working", stats(150, 20, 0, 0));
    poll(&mut g, &mut h, 5_000);
    does(&mut h, "local-turn", "starting", stats(300, 40, 0, 0));
    poll(&mut g, &mut h, 6_000);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 340)]);
    poll(&mut g, &mut h, 7_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

#[test]
fn ten_sessions_that_work_cost_ten_calls_a_poll_and_idle_ones_cost_none() {
    let (mut g, mut h) = start();
    for at in 0..10 {
        does(
            &mut h,
            &format!("local-w{at}"),
            "working",
            stats(1, 1, 0, 0),
        );
        does(&mut h, &format!("local-i{at}"), "idle", stats(1, 1, 0, 0));
    }
    poll(&mut g, &mut h, 1_000);
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h).len(), 10);
    let lists = |step: &&Step| matches!(step, Step::List(_));
    assert_eq!(h.asked.iter().filter(lists).count(), 2);
}

#[test]
fn a_session_that_stopped_working_is_asked_once_more_and_then_left() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 120)]);
    // The turn ended, and its last call added to the count after the
    // poll before looked.
    does(&mut h, "local-turn", "idle", stats(200, 40, 0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 240)]);
    // Idle at the poll before too: not asked, and no sample is given
    // again from an older poll.
    poll(&mut g, &mut h, 3_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    assert_eq!(samples(&h), []);
    // The same where it stopped to wait on a question.
    does(&mut h, "local-turn", "working", stats(300, 40, 0, 0));
    poll(&mut g, &mut h, 4_000);
    does(&mut h, "local-turn", "waiting", stats(310, 40, 0, 0));
    poll(&mut g, &mut h, 5_000);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 350)]);
    poll(&mut g, &mut h, 6_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

#[test]
fn a_gateway_of_this_machine_asks_no_stats() {
    let mut g = local();
    identified(&mut g, "one");
    let mut h = Hermes::default();
    does(&mut h, "local-works", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    assert!(g.count().is_none());
}

#[test]
fn zeros_a_failed_call_and_a_reply_that_is_no_count_give_no_sample() {
    let (mut g, mut h) = start();
    // Hermes answers zeros where it holds no agent for the session.
    does(&mut h, "local-a-zero", "working", stats(0, 0, 0, 0));
    does(&mut h, "local-b-fails", "working", stats(5, 5, 0, 0));
    does(
        &mut h,
        "local-c-text",
        "working",
        json!({"input_tokens": "many"}),
    );
    does(
        &mut h,
        "local-d-part",
        "working",
        json!({"output_tokens": 7}),
    );
    does(
        &mut h,
        "local-e-less",
        "working",
        json!({"input_tokens": -4}),
    );
    // One count that is no whole number spoils the reply, whatever the
    // others say.
    let mixed = json!({"input_tokens": "many", "output_tokens": 7});
    does(&mut h, "local-f-mixed", "working", mixed);
    let broken = json!({"input_tokens": 5, "cache_read_tokens": 1.5});
    does(&mut h, "local-g-part", "working", broken);
    let fails = g.session(&Profile::Default, "local-b-fails").unwrap();
    h.failing.push(Step::Stats(fails));
    poll(&mut g, &mut h, 1_000);
    assert_eq!(asked(&h).len(), 7);
    // A count Hermes left out is none of that kind.
    assert_eq!(samples(&h), [("local-d-part".to_owned(), 7)]);
    // A call that failed is asked again at the next poll.
    poll(&mut g, &mut h, 2_000);
    assert_eq!(
        samples(&h),
        [
            ("local-b-fails".to_owned(), 10),
            ("local-d-part".to_owned(), 7)
        ]
    );
}

#[test]
fn only_what_the_newest_poll_that_counted_listed_is_asked() {
    let (mut g, mut h) = start();
    does(&mut h, "local-works", "working", stats(100, 20, 0, 0));
    // A poll that is no longer the newest when it comes back is dropped:
    // nothing is listed, and nobody is asked.
    let mut stale = g.poll().unwrap();
    ask(&mut stale, &mut h);
    let _newer = g.poll().unwrap();
    g.apply(Input::Polled(stale), 500);
    assert!(g.count().is_none());

    poll(&mut g, &mut h, 1_000);
    assert_eq!(samples(&h), [("local-works".to_owned(), 120)]);
    // The socket is lost: nobody is asked.
    g.apply(Input::Lost(Failure::Unreachable), 1_500);
    assert!(g.count().is_none());
    // Back, and nobody is asked before a poll has listed anew.
    identified(&mut g, "one");
    assert!(g.count().is_none());
    // The first poll asks one that worked when the socket was lost and
    // has stopped since.
    does(&mut h, "local-works", "idle", stats(300, 60, 0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-works"]);
    assert_eq!(samples(&h), [("local-works".to_owned(), 360)]);
}

#[test]
fn a_profile_whose_list_failed_gives_no_sample() {
    let (mut g, mut h) = start();
    does(&mut h, "local-works", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    h.failing.push(Step::List(Profile::Default));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    assert!(h.counted.is_empty()); // Nor is one that is owed a count, until its profile is listed
                                   // again.
    does(&mut h, "local-works", "idle", stats(300, 60, 0, 0));
    let at = g.session(&Profile::Default, "local-works").unwrap();
    h.failing.push(Step::Stats(at));
    poll(&mut g, &mut h, 3_000);
    assert_eq!(asked(&h), ["local-works"]);
    h.failing.push(Step::List(Profile::Default));
    poll(&mut g, &mut h, 4_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    poll(&mut g, &mut h, 5_000);
    assert_eq!(samples(&h), [("local-works".to_owned(), 360)]);
}

/// Count calls are no step of a poll (ticket 60): a poll of a far
/// gateway whose sessions work asks the list and the reads, and ends
/// without one `session.stats`. They are a round of their own.
#[test]
fn no_poll_asks_a_count() {
    let (mut g, mut h) = start();
    for id in ["local-a", "local-b", "local-c"] {
        does(&mut h, id, "working", stats(100, 20, 0, 0));
    }
    poll(&mut g, &mut h, 1_000);
    let mut next = g.poll().unwrap();
    ask(&mut next, &mut h);
    assert_eq!(asked(&h), [] as [&str; 0], "{:?}", h.asked);
    assert!(next.steps().is_empty());
    // A reply to one given to a poll all the same changes nothing of it.
    let at = g.session(&Profile::Default, "local-a").unwrap();
    next.answer(&Step::Stats(at.clone()), &stats(1, 1, 0, 0));
    next.failed(&Step::Stats(at));
    assert!(next.steps().is_empty());
    g.apply(Input::Polled(next), 2_000);
    // The round after it asks each session that works, once.
    count(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-a", "local-b", "local-c"]);
}

/// The call at a turn's end fails (ticket 60, the verifier's probe A):
/// the session is asked again in the next round, though it is idle by
/// then, until a call is answered. Then it is left.
#[test]
fn a_last_count_whose_call_failed_is_asked_again_until_it_is_answered() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    does(&mut h, "local-turn", "idle", stats(300, 60, 0, 0));
    let at = g.session(&Profile::Default, "local-turn").unwrap();
    for now in [2_000, 3_000] {
        h.failing.push(Step::Stats(at.clone()));
        poll(&mut g, &mut h, now);
        assert_eq!(asked(&h), ["local-turn"]);
        assert_eq!(samples(&h), []);
    }
    poll(&mut g, &mut h, 13_000);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 360)]);
    poll(&mut g, &mut h, 23_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

/// A session whose call never answers is not asked for ever: five
/// rounds, and it is left until a turn runs in it again.
#[test]
fn a_last_count_that_never_answers_is_given_up_after_five_rounds() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    does(&mut h, "local-turn", "idle", stats(300, 60, 0, 0));
    let at = g.session(&Profile::Default, "local-turn").unwrap();
    let mut rounds = 0;
    for round in 0..8 {
        h.failing.push(Step::Stats(at.clone()));
        poll(&mut g, &mut h, 2_000 + round * 10_000);
        rounds += asked(&h).len();
        h.failing.clear();
    }
    assert_eq!(rounds, 5);
    // Zeros are an answer: Hermes holds no agent for it. Asked once.
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 90_000);
    does(&mut h, "local-turn", "idle", stats(0, 0, 0, 0));
    poll(&mut g, &mut h, 91_000);
    assert_eq!(asked(&h), ["local-turn"]);
    poll(&mut g, &mut h, 92_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

/// A turn that ran wholly between two polls that both list the session
/// as idle (probe B): the poll's look at the stream reads that a turn
/// started and ended (`turns.rs`), and the session is asked once.
#[test]
fn a_turn_that_ran_wholly_between_two_idle_polls_is_counted_once() {
    // The tests of the cards run again with the looks forced off.
    if std::env::var(super::super::turns::LOOKS).as_deref() == Ok("off") {
        return;
    }
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "idle", stats(100, 20, 0, 0));
    h.publish(
        &Profile::Default,
        "local-turn",
        "message.start",
        1,
        json!({}),
    );
    h.publish(
        &Profile::Default,
        "local-turn",
        "message.complete",
        1,
        json!({}),
    );
    h.stream(&Profile::Default, "local-turn").status = Some("idle".into());
    poll(&mut g, &mut h, 1_000);
    poll(&mut g, &mut h, 11_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    // The turn: it starts, uses, and ends before the next poll.
    h.publish(
        &Profile::Default,
        "local-turn",
        "message.start",
        2,
        json!({}),
    );
    h.publish(
        &Profile::Default,
        "local-turn",
        "message.complete",
        2,
        json!({}),
    );
    h.stream(&Profile::Default, "local-turn").status = Some("idle".into());
    h.stream(&Profile::Default, "local-turn").stats = stats(900, 90, 0, 0);
    poll(&mut g, &mut h, 21_000);
    assert_eq!(h.told.len(), 2, "the turn was read: {:?}", h.told);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 990)]);
    poll(&mut g, &mut h, 31_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

/// Waiting, then idle (probe C): a session that waited on a card and
/// whose turn ended before a poll saw it work again is asked once.
#[test]
fn a_session_that_waited_and_is_idle_now_is_counted_once() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    does(&mut h, "local-turn", "waiting", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-turn"]);
    poll(&mut g, &mut h, 3_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    does(&mut h, "local-turn", "idle", stats(400, 80, 0, 0));
    poll(&mut g, &mut h, 4_000);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 480)]);
    poll(&mut g, &mut h, 5_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
}

/// A round that was out while its session came to be owed a count says
/// nothing of that: what it read, it read before the turn ended.
#[test]
fn a_session_owed_a_count_while_a_round_is_out_is_asked_again() {
    let (mut g, mut h) = start();
    does(&mut h, "local-turn", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    // A round begins while it works, and is answered.
    let mut out = g.count().unwrap();
    let step = out.steps().remove(0);
    out.answer(&step, &stats(100, 20, 0, 0));
    // Meanwhile a poll finds it idle.
    does(&mut h, "local-turn", "idle", stats(300, 60, 0, 0));
    let mut next = g.poll().unwrap();
    ask(&mut next, &mut h);
    g.apply(Input::Polled(next), 2_000);
    g.apply(Input::Counted(out), 2_001);
    count(&mut g, &mut h, 2_002);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 360)]);
    // And one owed anew while the round that asked it was out.
    does(&mut h, "local-turn", "working", stats(300, 60, 0, 0));
    poll(&mut g, &mut h, 3_000);
    does(&mut h, "local-turn", "idle", stats(300, 60, 0, 0));
    let mut next = g.poll().unwrap();
    ask(&mut next, &mut h);
    g.apply(Input::Polled(next), 4_000);
    let mut out = g.count().unwrap();
    let step = out.steps().remove(0);
    out.answer(&step, &stats(300, 60, 0, 0));
    does(&mut h, "local-turn", "waiting", stats(300, 60, 0, 0));
    poll_alone(&mut g, &mut h, 5_000);
    does(&mut h, "local-turn", "idle", stats(500, 60, 0, 0));
    poll_alone(&mut g, &mut h, 6_000);
    g.apply(Input::Counted(out), 6_001);
    count(&mut g, &mut h, 6_002);
    assert_eq!(samples(&h), [("local-turn".to_owned(), 560)]);
}

/// One whole poll with no round of count calls after it.
fn poll_alone(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) {
    let mut poll = gateway.poll().unwrap();
    ask(&mut poll, hermes);
    gateway.apply(Input::Polled(poll), now_ms);
}

/// The cards are the same whether the stats calls are answered, fail or
/// are never asked: a count decides no card.
#[test]
fn a_count_decides_no_card() {
    let world = |hermes: &mut Hermes| {
        let asked = json!({"kind": "clarify", "prompt_id": "p1", "execution_generation": 1,
            "question": "Which one?", "choices": ["a", "b"], "multi_select": false});
        hermes.publish(
            &Profile::Default,
            "local-asks",
            "message.start",
            1,
            json!({}),
        );
        hermes.publish(&Profile::Default, "local-asks", "clarify.request", 1, asked);
        hermes.publish(
            &Profile::Default,
            "local-works",
            "message.start",
            1,
            json!({}),
        );
        hermes.stream(&Profile::Default, "local-works").stats = stats(100, 20, 0, 0);
    };
    let said = |effects: Vec<Effect>| format!("{effects:?}");
    let run = |mut gateway: Gateway, failing: bool| {
        identified(&mut gateway, "one");
        let mut hermes = Hermes::default();
        world(&mut hermes);
        let mut all = Vec::new();
        for now in [1_000, 2_000] {
            if failing {
                let works = gateway.session(&Profile::Default, "local-works").unwrap();
                hermes.failing.push(Step::Stats(works));
            }
            all.push(said(poll(&mut gateway, &mut hermes, now)));
        }
        // The question is answered elsewhere: its card closes.
        let settled = json!({"prompt_id": "p1"});
        hermes.publish(
            &Profile::Default,
            "local-asks",
            "clarify.settled",
            1,
            settled,
        );
        all.push(said(poll(&mut gateway, &mut hermes, 3_000)));
        (all, names(&gateway))
    };
    let far_id = || GatewayId::new(far(INSTALL), Profile::Default);
    let answered = run(Gateway::new(far_id()), false);
    let failed = run(Gateway::new(far_id()), true);
    assert_eq!(answered, failed);
    assert!(answered.0[0].contains("Open"), "{:?}", answered.0);
    assert!(answered.0[2].contains("Close"), "{:?}", answered.0);
    // A gateway of this machine, which asks no stats, opens and closes
    // the same card. Its ids name this machine, so the words differ.
    let here = run(local(), false);
    let shape = |all: &[String]| {
        let kinds = |text: &String| {
            (
                text.matches("Open(").count(),
                text.matches("Close {").count(),
            )
        };
        all.iter().map(kinds).collect::<Vec<_>>()
    };
    assert_eq!(shape(&here.0), shape(&answered.0));
    assert_eq!(here.1, answered.1);
}

/// What Hermes 0.22.5 answered in the lab, read as it is.
#[test]
fn the_recorded_stats_replies_are_read() {
    let recorded = fixture("session-stats");
    assert_eq!(
        Sample::of(&recorded["counting"]),
        Some(Sample {
            input: 100,
            output: 20,
            cache_read: 0,
            cache_write: 0
        })
    );
    assert_eq!(Sample::of(&recorded["no_agent"]), None);
    // The call, on a host's socket and on a profile's own.
    let (g, _) = start();
    let at = g
        .session(&Profile::named("verify").unwrap(), "local-a")
        .unwrap();
    let step = Step::Stats(at);
    let host = step.command(Scope::Host);
    assert_eq!(host.method, "session.stats");
    assert_eq!(
        host.params,
        json!({"session_id": "local-a", "profile": "verify"})
    );
    assert_eq!(
        step.command(Scope::Profile).params,
        json!({"session_id": "local-a"})
    );
}
