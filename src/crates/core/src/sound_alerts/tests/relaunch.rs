//! What the tracker decided is kept in the data folder across a relaunch
//! (`kept.rs`): a request decided before the app quit does not sound
//! after it, however late its card comes back.
use super::*;

/// A card of a session whose client the app reaches only after it has
/// started, as a session on another machine is.
fn late(session: &str, request: &str) -> Value {
    waiting(session, &[(request, "question")])
}

/// A tracker that keeps what it decides in `file`, past its first look.
fn launched(file: &std::path::Path, now: i64) -> Tracker {
    let mut tracker = Tracker::kept(file.to_owned(), now);
    tracker.update(&no_events(), &[], &Heard::default(), &on(), now);
    tracker
}

#[test]
fn a_request_decided_before_a_relaunch_does_not_sound_after_it() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut tracker = launched(&file, T);
    let card = late("far-1", "native:question:1");
    let alerts = look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&card),
        &on(),
        T + 1_000,
    );
    assert_eq!(alerts, [needs_you("far-1")]);
    drop(tracker);

    // Relaunched. The first look sees nothing waiting: the client is not
    // reached yet. Its card arrives twenty seconds later.
    let mut again = launched(&file, T + 60_000);
    for second in 1..20 {
        let alerts = look(
            &mut again,
            &no_events(),
            &[],
            &on(),
            T + 60_000 + second * 1_000,
        );
        assert!(alerts.is_empty());
    }
    let alerts = look(&mut again, &no_events(), &[card], &on(), T + 80_000);
    assert!(alerts.is_empty(), "{alerts:?}");

    // A question asked while the app was closed was never decided: it
    // sounds when its card arrives.
    let asked_meanwhile = late("far-1", "native:question:2");
    let alerts = look(
        &mut again,
        &no_events(),
        &[asked_meanwhile],
        &on(),
        T + 81_000,
    );
    assert_eq!(alerts, [needs_you("far-1")]);
}

#[test]
fn without_the_file_a_late_card_sounds_again() {
    // What the fix is for: a tracker that starts empty takes the card
    // that comes after its first look for a new one.
    let mut tracker = started();
    let card = late("far-1", "native:question:1");
    assert_eq!(
        look(
            &mut tracker,
            &no_events(),
            std::slice::from_ref(&card),
            &on(),
            T + 1_000
        )
        .len(),
        1
    );
    let mut again = Tracker::new();
    again.update(&no_events(), &[], &Heard::default(), &on(), T + 60_000);
    assert_eq!(
        look(&mut again, &no_events(), &[card], &on(), T + 80_000),
        [needs_you("far-1")]
    );
}

#[test]
fn decided_identities_older_than_a_day_are_dropped_at_load() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut tracker = launched(&file, T);
    let old = late("far-1", "native:question:old");
    look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&old),
        &on(),
        T + 1_000,
    );
    // The old one is answered; a newer one waits until the app quits.
    let new = late("far-2", "native:question:new");
    let at = T + 3_600_000;
    look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&new),
        &on(),
        at,
    );
    drop(tracker);

    // A day after the old one was last seen, less a second: both are kept.
    let kept = Tracker::kept(file.clone(), T + 1_000 + DAY - 1_000);
    assert_eq!(kept.decided.len(), 2);
    // A second past the day: the old one is dropped, the newer one kept.
    let now = T + 1_000 + DAY + 1_000;
    let mut again = Tracker::kept(file.clone(), now);
    let ids: Vec<&String> = again.decided.keys().collect();
    assert_eq!(ids, ["needs-you:claude:far-2:native:question:new"]);
    again.update(&no_events(), &[], &Heard::default(), &on(), now);
    // So the old request, were it still to wait, sounds; the newer does not.
    assert_eq!(
        look(&mut again, &no_events(), &[old], &on(), now + 1_000),
        [needs_you("far-1")]
    );
    assert!(look(&mut again, &no_events(), &[new], &on(), now + 2_000).is_empty());
}

#[test]
fn a_finished_turn_decided_before_a_relaunch_stays_decided() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut tracker = launched(&file, T);
    let island = completed("far-1", "epoch-1:7");
    let alerts = look(&mut tracker, &island, &[], &on(), T + 1_000);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].0, "finished");
    drop(tracker);
    let mut again = launched(&file, T + 60_000);
    assert!(look(&mut again, &island, &[], &on(), T + 61_000).is_empty());
}

#[test]
fn the_file_is_written_when_something_new_is_decided_and_seldom_otherwise() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut tracker = launched(&file, T);
    // Nothing decided, nothing written.
    assert!(!file.exists());
    let card = late("far-1", "native:question:1");
    look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&card),
        &on(),
        T + 1_000,
    );
    let written = std::fs::read(&file).unwrap();
    let saved: Value = serde_json::from_slice(&written).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(
        saved["decided"]["needs-you:claude:far-1:native:question:1"],
        T + 1_000
    );
    // It names sessions and requests, and holds no word anyone wrote.
    assert_eq!(saved.as_object().unwrap().len(), 2);

    // The same request, looked at once a second: its time moves on at
    // every look, and the file is not written for that until ten
    // minutes have gone by.
    for second in 2..600 {
        look(
            &mut tracker,
            &no_events(),
            std::slice::from_ref(&card),
            &on(),
            T + second * 1_000,
        );
    }
    assert_eq!(std::fs::read(&file).unwrap(), written);
    look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&card),
        &on(),
        T + 601_000,
    );
    let later: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        later["decided"]["needs-you:claude:far-1:native:question:1"],
        T + 601_000
    );

    // One gone for a day is forgotten, in the file too.
    look(&mut tracker, &no_events(), &[], &on(), T + 601_000 + DAY);
    let empty: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(empty["decided"], json!({}));
}

#[test]
fn a_file_that_cannot_be_used_is_an_empty_one() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    for bad in [
        "",
        "not json",
        "[]",
        r#"{"version":2,"decided":{"needs-you:claude:far-1:x":1800000000000}}"#,
        r#"{"version":1,"decided":["needs-you:claude:far-1:x"]}"#,
        r#"{"version":1,"decided":{"needs-you:claude:far-1:x":"soon"}}"#,
    ] {
        std::fs::write(&file, bad).unwrap();
        let tracker = Tracker::kept(file.clone(), T);
        assert!(tracker.decided.is_empty(), "{bad}");
    }
    // A time from a clock that ran ahead is taken as now, not kept for ever.
    let ahead = format!(
        r#"{{"version":1,"decided":{{"needs-you:claude:far-1:x":{}}}}}"#,
        T + 10 * DAY
    );
    std::fs::write(&file, ahead).unwrap();
    let tracker = Tracker::kept(file.clone(), T);
    assert_eq!(tracker.decided["needs-you:claude:far-1:x"], T);
    // A folder that is not there: nothing is kept, and nothing fails.
    let nowhere = folder.path().join("gone").join("sound-alerts.json");
    let mut tracker = launched(&nowhere, T);
    let card = late("far-1", "native:question:1");
    assert_eq!(
        look(&mut tracker, &no_events(), &[card], &on(), T + 1_000),
        [needs_you("far-1")]
    );
}

#[test]
fn with_sound_off_what_was_decided_is_kept_all_the_same() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut tracker = Tracker::kept(file.clone(), T);
    tracker.update(&no_events(), &[], &Heard::default(), &off(), T);
    let card = late("far-1", "native:question:1");
    assert!(look(
        &mut tracker,
        &no_events(),
        std::slice::from_ref(&card),
        &off(),
        T + 1_000
    )
    .is_empty());
    drop(tracker);
    // Turned on after a relaunch: what waited before is not told.
    let mut again = launched(&file, T + 60_000);
    assert!(look(&mut again, &no_events(), &[card], &on(), T + 80_000).is_empty());
}
