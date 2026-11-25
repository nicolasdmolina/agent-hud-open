//! Random lives of one session, seeded (Agent HUD Fork, ticket 75; the
//! round 2 verifier's harness, `evidence/75/verify2/w75.rs`). Each look
//! moves the clock by a second to three days, may start or end a wait
//! (told as one of the kinds of the mode, and with something written or
//! not), adds other alerts so the first ones leave the list, turns sound
//! off for a look, or relaunches the app from its saved file.
//!
//! A wait is new when a look sees the session wait and the look before,
//! in the same run of the app, did not. Every new wait heard with sound
//! on is heard, but one rule of ticket 66: a card of a request decided
//! within the day before is that request (`CARD`), and is silent. No
//! other look plays anything: a wait that goes on, across sleeps, sound
//! off and on, alerts leaving the list and relaunches, sounds once.
use super::lab::*;
use super::*;

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) % n
    }
}

#[derive(Default, Debug)]
struct Counts {
    new: usize,
    heard: usize,
    same_request: usize,
    silent: usize,
    extra: usize,
    most_counts: usize,
    most_bytes: usize,
}

const LOOKS: usize = 150;

fn live(
    mode: &str,
    kinds: &[Kind],
    writes: bool,
    seed: u64,
    folder: &std::path::Path,
    counts: &mut Counts,
) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let file = folder.join(format!("{mode}-{seed}.json"));
    let mut lab = Lab::kept(&file, T);
    lab.idle(0);
    let (mut waits, mut was, mut relaunched, mut kind, mut written) =
        (false, false, false, kinds[0], 0);
    for _ in 0..LOOKS {
        let gap = match rng.below(100) {
            0..=39 => 1_000,
            40..=64 => HOUR,
            65..=79 => 6 * HOUR,
            80..=94 => DAY + 1,
            _ => 3 * DAY,
        };
        if rng.below(100) < 35 {
            waits = !waits;
            if waits {
                kind = kinds[rng.below(kinds.len() as u64) as usize];
            } else if writes && rng.below(100) < 30 {
                written += 1;
            }
        }
        let others = [0, 0, 0, 5, 40][rng.below(5) as usize];
        let sound = rng.below(100) >= 4;
        if rng.below(100) < 3 {
            lab = Lab::kept(&file, lab.now);
            relaunched = true;
        }
        let since = format!(
            "2027-01-15T08:{:02}:{:02}Z",
            written / 60 % 60,
            written % 60
        );
        let row = if waits {
            row(kind, Some(&since))
        } else {
            running("s1")
        };
        let at = lab.now + gap;
        let decided = lab.tracker.decided.get(CARD).is_some_and(|t| at - t < DAY);
        let pings = lab.see(gap, Some(row), kind, others, sound);
        if waits && !was && !relaunched && sound {
            counts.new += 1;
            match (pings, kind == Kind::Card && decided) {
                (0, true) => counts.same_request += 1,
                (0, false) => counts.silent += 1,
                _ => counts.heard += 1,
            }
        } else {
            counts.extra += pings;
        }
        (was, relaunched) = (waits, false);
        let held = lab.held();
        counts.most_counts = counts.most_counts.max(held.0);
        counts.most_bytes = counts.most_bytes.max(held.1);
    }
}

#[test]
fn every_new_wait_is_heard_and_a_wait_that_goes_on_sounds_once() {
    let folder = tempfile::tempdir().unwrap();
    let modes: [(&str, &[Kind], bool); 7] = [
        ("bare", &[Kind::Bare], false),
        ("named", &[Kind::Named], false),
        ("ask", &[Kind::Ask], false),
        ("mixed3", &NO_CARD, false),
        ("mixed3-writes", &NO_CARD, true),
        ("card", &[Kind::Card], false),
        (
            "mixed4",
            &[Kind::Bare, Kind::Named, Kind::Ask, Kind::Card],
            false,
        ),
    ];
    for (mode, kinds, writes) in modes {
        let mut counts = Counts::default();
        for seed in 1..=80 {
            live(mode, kinds, writes, seed, folder.path(), &mut counts);
        }
        println!("lives {mode}: {counts:?}");
        assert_eq!((counts.silent, counts.extra), (0, 0), "{mode}: {counts:?}");
        assert!(
            counts.heard > 0 && counts.most_counts <= 16,
            "{mode}: {counts:?}"
        );
    }
}
