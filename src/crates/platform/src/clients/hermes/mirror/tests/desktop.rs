//! The sessions the Hermes desktop app hosts on a far machine (ticket
//! 39): the refresh's last read says the desktop app holds them, and no
//! poll since lists them live. A session that is both held and listed is
//! the gateway's, and is not one.
use super::*;
use crate::clients::hermes::mirror::{Batch, Wrote};

/// The refresh's word, read at `read_ms` under route `route`: the desktop
/// app holds `held` in `profile`, where it was read at all.
fn holds(bench: &Bench, profile: &Profile, held: Option<&[&str]>, route: u64) -> Wrote {
    bench.writer.history(Batch {
        profile: profile.clone(),
        sessions: Vec::new(),
        held: held.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect()),
        reads: None,
        read_ms: crate::hooks::now_ms(),
        route,
    })
}

fn hosted(bench: &Bench, fresh_ms: i64) -> Vec<(Profile, String)> {
    bench.writer.desktop_hosted(fresh_ms)
}

fn one(profile: &Profile, id: &str) -> Vec<(Profile, String)> {
    vec![(profile.clone(), id.to_owned())]
}

#[test]
fn a_session_the_desktop_app_holds_that_no_poll_lists_is_hosted_there() {
    let bench = bench("Mac mini");
    let (default, reports) = (Profile::Default, reports());
    let now = crate::hooks::now_ms();
    let fresh = now - 120_000;
    assert!(hosted(&bench, fresh).is_empty());

    // Held: `d1` and `g1`. A poll lists `g1` live: it is the gateway's.
    bench.lists(&default, &[("g1", "Gateway")], now, 1);
    assert_eq!(
        holds(&bench, &default, Some(&["d1", "g1"]), 1),
        Wrote::Written(0)
    );
    assert_eq!(hosted(&bench, fresh), one(&default, "d1"));

    // A read that did not read the held file leaves the word as it was.
    assert_eq!(holds(&bench, &default, None, 1), Wrote::Written(0));
    assert_eq!(hosted(&bench, fresh), one(&default, "d1"));
    // A word read under a route that is not the machine's now is dropped.
    assert_eq!(
        holds(&bench, &default, Some(&[]), 2),
        Wrote::Dropped("route")
    );
    assert_eq!(hosted(&bench, fresh), one(&default, "d1"));

    // `d1` joins the live list: held and listed, the gateway's.
    bench.lists(
        &default,
        &[("g1", "Gateway"), ("d1", "Desktop")],
        now + 1,
        1,
    );
    assert!(hosted(&bench, fresh).is_empty());
    // A list older than `fresh_ms` says nothing of what is live now.
    let later = now + 2;
    assert_eq!(
        hosted(&bench, later),
        [
            (default.clone(), "d1".into()),
            (default.clone(), "g1".into())
        ]
    );

    // Each profile is its own: listed in one profile is not in another's.
    assert_eq!(holds(&bench, &reports, Some(&["d1"]), 1), Wrote::Written(0));
    assert_eq!(hosted(&bench, fresh), one(&reports, "d1"));

    // Held no more: an ordinary stored session.
    assert_eq!(holds(&bench, &reports, Some(&[]), 1), Wrote::Written(0));
    assert!(hosted(&bench, fresh).is_empty());

    // A stopped writer knows of none.
    assert_eq!(holds(&bench, &default, Some(&["d9"]), 1), Wrote::Written(0));
    assert_eq!(hosted(&bench, fresh), one(&default, "d9"));
    bench.writer.stop();
    assert!(hosted(&bench, fresh).is_empty());
}
