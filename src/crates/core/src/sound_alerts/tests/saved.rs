//! What the counts of ended waits (`bare.rs`), the open waits (`open.rs`)
//! and the alert ids do across a relaunch and an upgrade (Agent HUD Fork,
//! ticket 75, round 3).
use super::lab::*;
use super::*;

const BASE: &str = "needs-you:claude:s1:2027-01-15T08:00:00Z";

/// A wait ends and the app quits at once, before the file's ten minutes:
/// the count is written when it changes, and the next wait sounds.
#[test]
fn a_wait_that_ended_just_before_a_relaunch_lets_the_next_sound() {
    let folder = tempfile::tempdir().unwrap();
    for kind in NO_CARD {
        let file = folder.path().join(format!("{kind:?}.json"));
        let mut lab = Lab::kept(&file, T);
        lab.idle(0);
        let first = lab.wait(1, kind);
        lab.run(100);
        let mut lab = Lab::kept(&file, lab.now + 1_000);
        lab.idle(0);
        assert_eq!((first, lab.wait(1, kind)), (1, 1), "{kind:?}");
    }
}

/// The app quits while a wait goes on; the wait ends while it is closed.
/// Every wait open at the quit is taken as over when the file is loaded,
/// so the next wait sounds. A wait still there at the first look is only
/// noted; one first seen later (its client reached late) is told again.
#[test]
fn a_wait_that_ended_while_the_app_was_closed_lets_the_next_sound() {
    let folder = tempfile::tempdir().unwrap();
    for kind in NO_CARD {
        let launch = |name: &str| {
            let file = folder.path().join(format!("{kind:?}-{name}.json"));
            let mut lab = Lab::kept(&file, T);
            lab.idle(0);
            let first = lab.wait(1, kind);
            lab.wait(1_000, kind);
            (first, Lab::kept(&file, lab.now + 60_000))
        };
        let (first, mut lab) = launch("ended");
        let (unlisted, at_work) = (lab.idle(0), lab.run(1_000));
        let next = lab.wait(1_000, kind);
        assert_eq!((first, unlisted, at_work, next), (1, 0, 0, 1), "{kind:?}");
        let (first, mut lab) = launch("on");
        let still = lab.wait(0, kind) + lab.wait(1_000, kind);
        assert_eq!((first, still), (1, 0), "{kind:?}");
        let (first, mut lab) = launch("late");
        let late = lab.idle(0) + lab.idle(20_000) + lab.wait(1_000, kind);
        assert_eq!((first, late), (1, 1), "{kind:?}");
    }
}

/// A file an earlier build saved marks a name as that of a wait seen to
/// end (`<name>/over`): it is taken as a count, and the next wait sounds.
#[test]
fn the_marks_an_earlier_build_saved_are_taken_as_counts() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let at = T - 1_000;
    let old = json!({"version":1,"decided":{
        BASE: at, format!("{BASE}/over"): at,
        format!("{BASE}+"): at, format!("{BASE}+/over"): at,
    }});
    std::fs::write(&file, old.to_string()).unwrap();
    let mut lab = Lab::kept(&file, T);
    lab.idle(0);
    assert_eq!(lab.wait(1, Kind::Bare), 1);
    assert_eq!(lab.tracker.ends[BASE].0, 2);
    assert!(lab.tracker.decided.keys().all(|id| !id.ends_with("/over")));
}

/// A name decided anew a day after its last wait, while that wait's alert
/// is still among those the snapshot carries, is told under another id.
#[test]
fn an_alert_is_told_under_an_id_the_snapshot_does_not_carry() {
    let mut lab = Lab::new();
    lab.wait(1, Kind::Bare);
    lab.run(100);
    lab.idle(DAY + 1);
    assert_eq!(lab.wait(1, Kind::Bare), 1);
    let ids: Vec<&str> = lab
        .tracker
        .items
        .iter()
        .filter_map(|i| i["id"].as_str())
        .collect();
    assert_eq!(ids, [BASE.to_owned(), format!("{BASE}~2")]);
}

/// The counts and open waits are saved only beside what is decided, and
/// a file with none has the two fields of an earlier build.
#[test]
fn the_file_holds_counts_and_open_waits_only_when_there_are_some() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("sound-alerts.json");
    let mut lab = Lab::kept(&file, T);
    lab.idle(0);
    lab.wait(1, Kind::Named);
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let open = json!([
        ["needs-you:claude:s1:req-1"],
        T + 1,
        ["needs-you:claude:s1:req-1"]
    ]);
    assert_eq!(
        (saved.get("ended"), &saved["open"]["claude:s1"]),
        (None, &open)
    );
    lab.run(100);
    let saved: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        saved["ended"]["needs-you:claude:s1:req-1"],
        json!([1, T + 101])
    );
    assert_eq!(saved.get("open"), None);
}

/// A card its channel does not name, of a request whose wait with no card
/// ended before it, sounds and goes on for over a day: it keeps the
/// request's count, and the request's next wait with no card sounds.
#[test]
fn a_card_keeps_the_count_of_its_request_alive() {
    let mut lab = Lab::new();
    let first = lab.wait(1, Kind::Named);
    lab.run(100);
    let card = || Some(row(Kind::Card, None));
    let shown: usize = (0..30)
        .map(|_| lab.see(HOUR, card(), Kind::Bare, 0, true))
        .sum();
    lab.run(100);
    let next = lab.wait(100, Kind::Named);
    assert_eq!((first, shown, next), (1, 1, 1));
}

/// A card that ended, and the same card three days later with no look
/// between: its name was last among the inputs before the look that saw
/// it gone, so it is forgotten before it is decided by, and it sounds.
#[test]
fn a_card_three_days_after_it_ended_sounds() {
    let mut lab = Lab::new();
    let first = lab.wait(1, Kind::Card);
    lab.run(HOUR);
    assert_eq!((first, lab.wait(3 * DAY, Kind::Card)), (1, 1));
}
