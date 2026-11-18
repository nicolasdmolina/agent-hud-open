//! The live list as rows (`listing.rs`): what a row of each live session
//! shows, from the newest poll that counted and from nothing older.
use super::super::*;
use super::fake::{identified, poll, Hermes};
use super::{far, INSTALL};
use serde_json::json;

fn verify() -> Profile {
    Profile::named("verify").unwrap()
}

fn start() -> (Gateway, Hermes) {
    let mut gateway = Gateway::new(GatewayId::new(far(INSTALL), Profile::Default));
    identified(&mut gateway, "one");
    (gateway, Hermes::default())
}

/// The sessions of `profile` in the listing, by id.
fn rows(gateway: &Gateway, profile: &Profile) -> Vec<(String, Listed)> {
    let listing = gateway.listing().expect("a listing");
    listing.profiles.get(profile).cloned().expect("the profile")
}

#[test]
fn a_listing_says_what_a_row_of_each_live_session_shows() {
    let (mut g, mut h) = start();
    let stream = h.stream(&Profile::Default, "local-chat");
    stream.status = Some("working".into());
    stream.row = json!({"title": "Plan the week", "model": "claude-test",
        "source": "telegram", "started_at": 1_760_000_000.5, "last_active": 1_760_000_100.0});
    // Idle, and Hermes says little of it: it is listed all the same.
    h.stream(&verify(), "local-quiet").row = json!({"title": "", "model": "", "source": null});
    poll(&mut g, &mut h, 7_000);

    let listing = g.listing().unwrap();
    assert_eq!(listing.read_ms, 7_000);
    assert_eq!(
        rows(&g, &Profile::Default),
        [(
            "local-chat".to_owned(),
            Listed {
                title: Some("Plan the week".into()),
                model: Some("claude-test".into()),
                source: Some("telegram".into()),
                started_at: Some(1_760_000_000.5),
            }
        )]
    );
    assert_eq!(
        rows(&g, &verify()),
        [("local-quiet".to_owned(), Listed::default())]
    );
}

#[test]
fn a_start_that_is_no_time_and_words_that_are_too_long_are_not_taken() {
    let (mut g, mut h) = start();
    let long = "t".repeat(5_000);
    for (id, started) in [
        ("local-zero", json!(0)),
        ("local-negative", json!(-4.0)),
        ("local-text", json!("yesterday")),
        ("local-whole", json!(1_760_000_000)),
    ] {
        h.stream(&Profile::Default, id).row =
            json!({"started_at": started, "title": long, "model": long, "source": long});
    }
    poll(&mut g, &mut h, 1);
    let rows = rows(&g, &Profile::Default);
    let started: Vec<_> = rows
        .iter()
        .map(|(id, row)| (id.as_str(), row.started_at))
        .collect();
    assert_eq!(
        started,
        [
            ("local-negative", None),
            ("local-text", None),
            ("local-whole", Some(1_760_000_000.0)),
            ("local-zero", None),
        ]
    );
    for (_, row) in rows {
        assert_eq!(row.title.unwrap().chars().count(), 512);
        assert_eq!(row.model.unwrap().chars().count(), 128);
        assert_eq!(row.source.unwrap().chars().count(), 128);
    }
}

#[test]
fn a_profile_with_no_session_is_listed_empty_and_one_whose_list_failed_is_left_out() {
    let (mut g, mut h) = start();
    h.stream(&verify(), "local-a");
    poll(&mut g, &mut h, 1_000);
    let listing = g.listing().unwrap();
    assert_eq!(listing.profiles[&Profile::Default], []);
    assert_eq!(listing.profiles[&verify()].len(), 1);

    // The list of `verify` fails: what it held is not known now, so it
    // is not in the listing, and the time is the newer poll's.
    h.failing.push(Step::List(verify()));
    poll(&mut g, &mut h, 2_000);
    let listing = g.listing().unwrap();
    assert_eq!(listing.read_ms, 2_000);
    assert!(!listing.profiles.contains_key(&verify()));
    assert!(listing.profiles.contains_key(&Profile::Default));

    // A session that left the list is in no later listing.
    h.streams.clear();
    poll(&mut g, &mut h, 3_000);
    assert_eq!(g.listing().unwrap().profiles[&verify()], []);
}

#[test]
fn nothing_is_listed_before_a_poll_counted_or_once_the_gateway_is_not_ready() {
    let (mut g, mut h) = start();
    h.stream(&Profile::Default, "local-a");
    assert!(g.listing().is_none(), "no poll yet");

    // A poll that is no longer the newest when it comes back is dropped:
    // nothing is listed from it.
    let mut stale = g.poll().unwrap();
    super::fake::ask(&mut stale, &mut h);
    let _newer = g.poll().unwrap();
    g.apply(Input::Polled(stale), 500);
    assert!(g.listing().is_none(), "a stale poll");

    poll(&mut g, &mut h, 1_000);
    assert!(g.listing().is_some());
    // The socket is lost: what the list said is not current.
    g.apply(Input::Lost(Failure::Unreachable), 1_500);
    assert!(g.listing().is_none(), "lost");
    // Back, and not polled yet: still nothing.
    identified(&mut g, "one");
    assert!(g.listing().is_none(), "ready and not polled");
    poll(&mut g, &mut h, 2_000);
    assert_eq!(g.listing().unwrap().read_ms, 2_000);
}

#[test]
fn only_a_session_in_a_turn_counts_as_working_however_many_are_listed() {
    let (mut g, mut h) = start();
    assert!(!g.working(), "no poll yet");
    // Twenty-five idle sessions, as a gateway that keeps its old chats
    // listed: all listed, none at work.
    for n in 0..25 {
        h.stream(&Profile::Default, &format!("idle-{n:02}"));
    }
    poll(&mut g, &mut h, 1_000);
    assert_eq!(rows(&g, &Profile::Default).len(), 25);
    assert!(!g.working());

    // One of them takes a turn, in either of the words Hermes has for it.
    for (word, at) in [("starting", 2_000), ("working", 3_000)] {
        h.stream(&Profile::Default, "idle-07").status = Some(word.into());
        poll(&mut g, &mut h, at);
        assert!(g.working(), "{word}");
    }
    // Waiting for an answer, its counts stand still: not at work.
    h.stream(&Profile::Default, "idle-07").status = Some("waiting".into());
    poll(&mut g, &mut h, 4_000);
    assert!(!g.working());
    // Back in its turn, and then done.
    h.stream(&Profile::Default, "idle-07").status = Some("working".into());
    poll(&mut g, &mut h, 5_000);
    assert!(g.working());
    h.stream(&Profile::Default, "idle-07").status = None;
    poll(&mut g, &mut h, 6_000);
    assert!(!g.working());

    // A working session of a profile whose list failed is not current.
    h.stream(&verify(), "busy").status = Some("working".into());
    poll(&mut g, &mut h, 7_000);
    assert!(g.working());
    h.failing.push(Step::List(verify()));
    poll(&mut g, &mut h, 8_000);
    assert!(!g.working());
    // And none is, of a gateway that is lost.
    h.failing.clear();
    poll(&mut g, &mut h, 9_000);
    assert!(g.working());
    g.apply(Input::Lost(Failure::Unreachable), 9_500);
    assert!(!g.working());
}

#[test]
fn a_session_that_waits_on_a_request_with_no_card_is_not_at_work() {
    let (mut g, mut h) = start();
    // Hermes lists a session in a turn that waits on a request of its
    // viewers as working: a set of questions does that, and so does a
    // request Agent HUD never shows. Neither shows a card here.
    for (n, open) in [
        json!({"id": "srq-1", "method": "secret", "params": {"prompt": "A key"}}),
        json!({"id": "srq-2", "method": "clarify", "params": {"questions": []}}),
    ]
    .into_iter()
    .enumerate()
    {
        let stream = h.stream(&Profile::Default, "asking");
        stream.status = Some("working".into());
        stream.open = vec![open];
        poll(&mut g, &mut h, 1_000 * (n as i64 + 1));
        assert_eq!(g.waiting().count(), 0, "no card");
        assert!(!g.working(), "it waits, and writes nothing meanwhile");
    }
    // A read of it that fails says nothing new: it still waits.
    let asking = g.session(&Profile::Default, "asking").unwrap();
    h.failing.push(Step::Since {
        session: asking,
        cursor: None,
    });
    poll(&mut g, &mut h, 3_000);
    assert!(!g.working());
    // Answered elsewhere: its turn goes on.
    h.stream(&Profile::Default, "asking").open.clear();
    poll(&mut g, &mut h, 4_000);
    assert!(g.working());
}
