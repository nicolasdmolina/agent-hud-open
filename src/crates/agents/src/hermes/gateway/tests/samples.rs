//! Live token counts of a far session (`sample.rs`): which sessions a
//! poll asks `session.stats` of, what a reply is taken for, and that the
//! samples of the newest poll that counted are given and no others.
use super::super::*;
use super::fake::{ask, identified, names, poll, Hermes};
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

/// The sessions the last poll asked `session.stats` of, in order.
fn asked(hermes: &Hermes) -> Vec<String> {
    let stats = |step: &Step| match step {
        Step::Stats(session) => Some(session.id().to_owned()),
        _ => None,
    };
    hermes.asked.iter().filter_map(stats).collect()
}

/// The samples of the default profile in the listing, by session.
fn samples(gateway: &Gateway) -> Vec<(String, u64)> {
    let listing = gateway.listing().expect("a listing");
    let of = listing.samples.get(&Profile::Default);
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
    assert_eq!(samples(&g), [("local-works".to_owned(), 155)]);
    let listing = g.listing().unwrap();
    assert_eq!(listing.read_ms, 1_000);
    assert_eq!(
        listing.samples[&Profile::Default][0].1,
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
    assert_eq!(samples(&g), [("local-turn".to_owned(), 120)]);
    poll(&mut g, &mut h, 4_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    // Working, then starting again, and starting still.
    does(&mut h, "local-turn", "working", stats(150, 20, 0, 0));
    poll(&mut g, &mut h, 5_000);
    does(&mut h, "local-turn", "starting", stats(300, 40, 0, 0));
    poll(&mut g, &mut h, 6_000);
    assert_eq!(samples(&g), [("local-turn".to_owned(), 340)]);
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
    assert_eq!(samples(&g), [("local-turn".to_owned(), 120)]);
    // The turn ended, and its last call added to the count after the
    // poll before looked.
    does(&mut h, "local-turn", "idle", stats(200, 40, 0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-turn"]);
    assert_eq!(samples(&g), [("local-turn".to_owned(), 240)]);
    // Idle at the poll before too: not asked, and no sample is given
    // again from an older poll.
    poll(&mut g, &mut h, 3_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    assert_eq!(samples(&g), []);
    // The same where it stopped to wait on a question.
    does(&mut h, "local-turn", "working", stats(300, 40, 0, 0));
    poll(&mut g, &mut h, 4_000);
    does(&mut h, "local-turn", "waiting", stats(310, 40, 0, 0));
    poll(&mut g, &mut h, 5_000);
    assert_eq!(samples(&g), [("local-turn".to_owned(), 350)]);
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
    assert!(g.listing().unwrap().samples.is_empty());
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
    assert_eq!(samples(&g), [("local-d-part".to_owned(), 7)]);
    // A call that failed is asked again at the next poll.
    poll(&mut g, &mut h, 2_000);
    assert_eq!(
        samples(&g),
        [
            ("local-b-fails".to_owned(), 10),
            ("local-d-part".to_owned(), 7)
        ]
    );
}

#[test]
fn only_the_newest_poll_that_counted_gives_samples() {
    let (mut g, mut h) = start();
    does(&mut h, "local-works", "working", stats(100, 20, 0, 0));
    // A poll that is no longer the newest when it comes back is dropped.
    let mut stale = g.poll().unwrap();
    ask(&mut stale, &mut h);
    let _newer = g.poll().unwrap();
    g.apply(Input::Polled(stale), 500);
    assert!(g.listing().is_none());

    poll(&mut g, &mut h, 1_000);
    assert_eq!(samples(&g), [("local-works".to_owned(), 120)]);
    // The socket is lost: nothing is current.
    g.apply(Input::Lost(Failure::Unreachable), 1_500);
    assert!(g.listing().is_none());
    // Back: the first poll asks the session that works again, and one
    // that worked when the socket was lost and has stopped since.
    identified(&mut g, "one");
    does(&mut h, "local-works", "idle", stats(300, 60, 0, 0));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), ["local-works"]);
    assert_eq!(samples(&g), [("local-works".to_owned(), 360)]);
    assert_eq!(g.listing().unwrap().read_ms, 2_000);
}

#[test]
fn a_profile_whose_list_failed_gives_no_sample() {
    let (mut g, mut h) = start();
    does(&mut h, "local-works", "working", stats(100, 20, 0, 0));
    poll(&mut g, &mut h, 1_000);
    h.failing.push(Step::List(Profile::Default));
    poll(&mut g, &mut h, 2_000);
    assert_eq!(asked(&h), [] as [&str; 0]);
    assert!(g.listing().unwrap().samples.is_empty());
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
