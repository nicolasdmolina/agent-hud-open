//! The wait after a long later wait (Agent HUD Fork, ticket 75, round 2;
//! the verifier's probes, `evidence/75/verify/v75.rs`). Each look's alerts
//! go through a copy of the host's filter (`host/src/system/sound.rs`,
//! `fresh`: an id plays unless the snapshot before carried it), so every
//! count here is what would be heard. A wait that goes on keeps the name it
//! was decided under, and the mark that sent it there stays while it is
//! remembered: the wait after it never lands on a name decided for it.
use super::*;

const HOUR: i64 = 3_600_000;
/// The three ways a wait with no card is told: by its waiting flag alone,
/// by a request its channel names, and by an ask.
const KINDS: [(&[&str], bool); 3] = [(&[], false), (&["req-1"], false), (&["req-1"], true)];

struct Lab {
    tracker: Tracker,
    played: HashSet<String>,
    now: i64,
}

impl Lab {
    fn new() -> Self {
        Self::from(started(), T)
    }
    fn from(tracker: Tracker, now: i64) -> Self {
        Self {
            tracker,
            played: HashSet::new(),
            now,
        }
    }
    /// One look `after` ms on; how many `needs-you` the host would play.
    fn at(
        &mut self,
        after: i64,
        island: &Value,
        rows: &[Value],
        waits: &[&str],
        asks: bool,
    ) -> usize {
        self.now += after;
        let (key, named) = (
            "claude:s1".to_owned(),
            waits.iter().map(|w| (*w).to_owned()),
        );
        let named: Vec<String> = named.collect();
        let asked = named.iter().map(|w| (w.clone(), w.clone())).collect();
        let heard = Heard {
            waits: HashMap::from([(key.clone(), named)]),
            asks: if asks {
                HashMap::from([(key, asked)])
            } else {
                HashMap::new()
            },
            ..Heard::default()
        };
        let out = self.tracker.update(island, rows, &heard, &on(), self.now);
        let mut seen = HashSet::new();
        let mut heard = 0;
        for item in out["items"].as_array().unwrap() {
            let id = item["id"].as_str().unwrap().to_owned();
            let fresh = seen.insert(id.clone()) && !self.played.contains(&id);
            heard += usize::from(fresh && item["kind"] == "needs-you");
        }
        self.played = seen;
        heard
    }
    fn wait(&mut self, after: i64, kind: (&[&str], bool)) -> usize {
        self.at(after, &no_events(), &[waiting("s1", &[])], kind.0, kind.1)
    }
    fn end(&mut self, after: i64, kind: (&[&str], bool)) -> usize {
        self.at(after, &no_events(), &[running("s1")], &[], kind.1)
    }
    fn marks(&self) -> usize {
        self.tracker
            .decided
            .keys()
            .filter(|k| k.ends_with("/over"))
            .count()
    }
}

/// `count` turn ends of other sessions, as the island tells them.
fn others(tag: &str, count: usize) -> Value {
    let items: Vec<Value> = (0..count).map(|n| json!({"id":format!("event:{tag}:{n}"),"kind":"completion","at":"2027-01-15T08:00:00Z","source":"codex","sessionId":format!("{tag}-{n}"),"turnId":format!("{tag}-{n}")})).collect();
    json!({"revision":1,"items":items})
}

/// Wait 1 sounds and ends; over a day later wait 2 sounds and goes on while
/// wait 1's alert leaves the 32 the snapshot carries; wait 2 ends and wait
/// 3 starts. Each wait sounds once.
#[test]
fn the_wait_after_one_that_went_on_while_the_first_alert_left_sounds() {
    for kind in KINDS {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.end(100, kind);
        lab.at(DAY + 1, &no_events(), &[], &[], kind.1);
        let second = lab.wait(1, kind);
        let mut extra = 0;
        for n in 0..40 {
            let rows = [waiting("s1", &[])];
            extra += lab.at(60_000, &others(&format!("o{n}"), 1), &rows, kind.0, kind.1);
        }
        lab.wait(100, kind);
        lab.end(100, kind);
        let third = lab.wait(100, kind);
        assert_eq!((first, second, extra, third), (1, 1, 0, 1), "{kind:?}");
    }
}

/// The same with sound turned off and on while wait 2 goes on, which
/// empties the list the snapshot carries.
#[test]
fn the_wait_after_one_that_went_on_through_sound_off_and_on_sounds() {
    let kind = KINDS[0];
    let mut lab = Lab::new();
    let first = lab.wait(1, kind);
    lab.end(100, kind);
    lab.at(DAY + 1, &no_events(), &[], &[], false);
    let second = lab.wait(1, kind);
    lab.now += 100;
    let rows = [waiting("s1", &[])];
    let heard = Heard::default();
    lab.tracker
        .update(&no_events(), &rows, &heard, &off(), lab.now);
    // A look with sound back on, and one more as the wait goes on.
    let back = lab.wait(100, kind) + lab.wait(100, kind);
    lab.end(100, kind);
    let third = lab.wait(100, kind);
    assert_eq!((first, second, back, third), (1, 1, 0, 1));
}

/// Wait 1 sounds and ends, wait 2 sounds at once and goes on for 30 hours,
/// ends, and wait 3 starts.
#[test]
fn the_wait_after_a_second_that_went_on_over_a_day_sounds() {
    for kind in KINDS {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.end(100, kind);
        let second = lab.wait(100, kind);
        let extra: usize = (0..30).map(|_| lab.wait(HOUR, kind)).sum();
        lab.end(100, kind);
        let third = lab.wait(100, kind);
        assert_eq!((first, second, extra, third), (1, 1, 0, 1), "{kind:?}");
    }
}

/// Waits 1 and 2 sound; the app relaunches while wait 2 goes on; wait 2
/// lasts 30 hours more and ends; wait 3 starts.
#[test]
fn the_wait_after_a_second_that_went_on_through_a_relaunch_sounds() {
    for kind in KINDS {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("sound-alerts.json");
        let mut lab = Lab::from(Tracker::kept(file.clone(), T), T);
        lab.at(0, &no_events(), &[], &[], kind.1);
        let first = lab.wait(1, kind);
        lab.end(100, kind);
        let second = lab.wait(100, kind);
        lab.wait(100, kind);
        let now = lab.now + 60_000;
        let mut lab = Lab::from(Tracker::kept(file, now), now);
        let again = lab.wait(0, kind);
        let extra: usize = (0..30).map(|_| lab.wait(HOUR, kind)).sum();
        lab.end(100, kind);
        let third = lab.wait(100, kind);
        assert_eq!(
            (first, second, again, extra, third),
            (1, 1, 0, 0, 1),
            "{kind:?}"
        );
    }
}

/// A request that waited with no card and ended, and its card a day later
/// while the first alert is still carried: another wait, and it sounds.
#[test]
fn a_card_a_day_after_the_wait_of_its_request_ended_sounds() {
    let kind = KINDS[1];
    let mut lab = Lab::new();
    let first = lab.wait(1, kind);
    lab.end(100, kind);
    lab.at(DAY + 1, &no_events(), &[], &[], false);
    let card = [waiting("s1", &[("req-1", "permission")])];
    let later = lab.at(1, &no_events(), &card, &["req-1"], false);
    let next = lab.at(1_000, &no_events(), &card, &["req-1"], false);
    assert_eq!((first, later, next), (1, 1, 0));
}

/// Memory: 600 waits in 300 sessions over 60 days, each ended, with turn
/// ends of other sessions. The marks held stay few, and a day after the
/// last none outlives its alerts.
#[test]
fn the_marks_held_stay_few_over_sixty_days() {
    let mut lab = Lab::new();
    let mut most = 0;
    for round in 0..600 {
        let id = format!("s{}", round % 300);
        lab.at(1_000, &no_events(), &[waiting(&id, &[])], &[], false);
        lab.at(1_000, &no_events(), &[running(&id)], &[], false);
        lab.at(
            DAY / 10,
            &others(&format!("r{round}"), round % 3),
            &[],
            &[],
            false,
        );
        most = most.max(lab.marks());
    }
    lab.at(DAY + 1, &no_events(), &[], &[], false);
    println!(
        "most marks held: {most}; a day after the last: {}",
        lab.marks()
    );
    assert!(most <= 32, "{most}");
    assert!(lab.marks() <= lab.tracker.items.len(), "{}", lab.marks());
}

/// Waits 1 and 2 sound and end; wait 3 sounds and goes on for 30 hours
/// while the first two alerts leave the list. It keeps the name it was
/// decided under all along and makes no other sound; wait 4 sounds.
#[test]
fn a_wait_that_goes_on_keeps_its_name_and_the_next_sounds() {
    for kind in KINDS {
        let mut lab = Lab::new();
        let first = lab.wait(1, kind);
        lab.end(100, kind);
        let second = lab.wait(100, kind);
        lab.end(100, kind);
        let third = lab.wait(100, kind);
        let name = |lab: &Lab| {
            let wait = &lab.tracker.waits["claude:s1"];
            let named = wait.named.iter().map(|(_, n)| n.clone());
            let asks = wait.asks.iter().map(|a| a.name.clone());
            (wait.lent.clone(), named.chain(asks).collect::<Vec<_>>())
        };
        let decided = name(&lab);
        let rows = [waiting("s1", &[])];
        let mut extra = 0;
        for n in 0..40 {
            extra += lab.at(60_000, &others(&format!("o{n}"), 1), &rows, kind.0, kind.1);
        }
        extra += (0..30).map(|_| lab.wait(HOUR, kind)).sum::<usize>();
        assert_eq!(name(&lab), decided, "{kind:?}");
        lab.end(100, kind);
        let fourth = lab.wait(100, kind);
        assert_eq!(
            (first, second, third, extra, fourth),
            (1, 1, 1, 0, 1),
            "{kind:?}"
        );
    }
}

/// A card a day after the wait of its request ended goes on while the
/// first alert leaves the list: one sound, not another when it leaves.
#[test]
fn a_card_that_goes_on_while_the_first_alert_leaves_sounds_once() {
    let kind = KINDS[1];
    let mut lab = Lab::new();
    lab.wait(1, kind);
    lab.end(100, kind);
    lab.at(DAY + 1, &no_events(), &[], &[], false);
    let card = [waiting("s1", &[("req-1", "permission")])];
    let later = lab.at(1, &no_events(), &card, &["req-1"], false);
    let mut extra = 0;
    for n in 0..40 {
        extra += lab.at(
            60_000,
            &others(&format!("o{n}"), 1),
            &card,
            &["req-1"],
            false,
        );
    }
    extra += lab.at(1_000, &no_events(), &card, &["req-1"], false);
    assert_eq!((later, extra), (1, 0));
}
