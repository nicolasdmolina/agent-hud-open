use super::*;
use serde_json::json;

fn snapshot(sound: bool, ids: &[(&str, &str)]) -> Value {
    let items = ids
        .iter()
        .map(|(id, kind)| json!({"id":id,"kind":kind,"source":"example","sessionId":"s1","sound":if *kind == "finished" {"Hero"} else {"Glass"},"at":1}))
        .collect::<Vec<_>>();
    json!({"settings":{"sound":sound},"soundAlerts":{"revision":items.len(),"items":items}})
}

fn ids(alerts: &[Alert]) -> Vec<&str> {
    alerts.iter().map(|alert| alert.id.as_str()).collect()
}

#[test]
fn each_alert_id_plays_once_with_its_kind_and_sound() {
    let mut played = HashSet::new();
    let first = fresh(&mut played, &snapshot(true, &[("a", "needs-you")]));
    assert_eq!(
        first,
        vec![Alert {
            id: "a".into(),
            kind: "needs-you".into(),
            sound: "Glass".into()
        }]
    );
    let again = snapshot(true, &[("a", "needs-you"), ("b", "finished")]);
    let second = fresh(&mut played, &again);
    assert_eq!(ids(&second), ["b"]);
    assert_eq!(second[0].sound, "Hero");
    assert!(fresh(&mut played, &again).is_empty());
}

#[test]
fn the_first_snapshot_plays_what_it_carries() {
    let mut played = HashSet::new();
    let alerts = fresh(
        &mut played,
        &snapshot(true, &[("a", "finished"), ("b", "needs-you")]),
    );
    assert_eq!(ids(&alerts), ["a", "b"]);
}

#[test]
fn nothing_plays_with_the_setting_off() {
    let mut played = HashSet::new();
    assert!(fresh(&mut played, &snapshot(false, &[("a", "finished")])).is_empty());
    // What arrived while off is not played later when the setting turns on.
    assert!(fresh(&mut played, &snapshot(true, &[("a", "finished")])).is_empty());
    let mut missing =
        json!({"soundAlerts":{"revision":1,"items":[{"id":"b","kind":"finished","sound":"Hero"}]}});
    assert!(fresh(&mut played, &missing).is_empty());
    missing["settings"] = json!({"sound":"yes"});
    assert!(fresh(&mut played, &missing).is_empty());
}

#[test]
fn a_restarted_core_counts_by_id_not_revision() {
    let mut played = HashSet::new();
    fresh(&mut played, &snapshot(true, &[("a", "finished")]));
    // A new core starts its revision over; its alerts have new ids.
    let restarted = snapshot(true, &[("c", "finished")]);
    assert_eq!(ids(&fresh(&mut played, &restarted)), ["c"]);
}

#[test]
fn a_snapshot_without_the_list_forgets_nothing() {
    let mut played = HashSet::new();
    fresh(&mut played, &snapshot(true, &[("a", "finished")]));
    assert!(fresh(&mut played, &json!({"settings":{"sound":true}})).is_empty());
    assert!(fresh(&mut played, &snapshot(true, &[("a", "finished")])).is_empty());
}

#[test]
fn a_repeated_id_in_one_snapshot_plays_once() {
    let mut played = HashSet::new();
    let alerts = fresh(
        &mut played,
        &snapshot(true, &[("a", "finished"), ("a", "finished")]),
    );
    assert_eq!(ids(&alerts), ["a"]);
}

#[test]
fn a_sound_is_a_plain_name() {
    for name in ["Glass", "Hero", "Sosumi", "my sound-2_b"] {
        assert!(plain(name), "{name}");
    }
    for name in ["", " ", "../Glass", "Glass.aiff", "a/b", "a\\b", "x\n"] {
        assert!(!plain(name), "{name:?}");
    }
}

#[test]
fn a_refused_name_is_logged_as_one_short_line_without_the_name() {
    let temp = tempfile::tempdir().unwrap();
    let sounds = Sounds::new(temp.path());
    assert!(sounds.preview("../Glass").is_err());
    assert!(sounds.preview("bad\nforged entry").is_err());
    assert!(sounds.preview(&"x".repeat(1 << 20)).is_err());
    let log = std::fs::read_to_string(temp.path().join("sound.log")).unwrap();
    assert_eq!(log.lines().count(), 3, "{log}");
    for line in log.lines() {
        assert!(
            line.ends_with(
                "failed kind=preview sound=<refused> error=A sound is a plain name, not a path"
            ),
            "{line}"
        );
    }
    assert!(!log.contains("forged") && !log.contains("Glass") && log.len() < 1024);
}

#[test]
fn a_line_is_one_line_and_bounded_whatever_it_holds() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("sound.log");
    record(&log, &format!("id=a\nb\r{}", "y".repeat(10_000)));
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.len() < LINE_LIMIT + 64, "{}", text.len());
}

#[test]
fn writers_at_once_keep_every_line_across_a_start_over() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("sound.log");
    for round in 0..20 {
        std::fs::write(&log, vec![b'.'; LOG_LIMIT as usize + 1]).unwrap();
        let writers = (0..30)
            .map(|writer| {
                let log = log.clone();
                std::thread::spawn(move || record(&log, &format!("round={round} writer={writer}")))
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = std::fs::read_to_string(&log).unwrap();
        for writer in 0..30 {
            assert!(
                text.contains(&format!("round={round} writer={writer}\n")),
                "round {round} lost writer {writer}"
            );
        }
        assert!(text.len() < LOG_LIMIT as usize);
    }
}
