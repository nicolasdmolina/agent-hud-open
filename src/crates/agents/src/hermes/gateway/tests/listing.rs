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
