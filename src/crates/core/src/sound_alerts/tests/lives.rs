//! Random lives of one session, seeded (Agent HUD Fork, ticket 75; the
//! round 2 verifier's harness, `evidence/75/verify2/w75.rs`). Each look
//! moves the clock by a second to three days, may start or end a wait
//! (told as one of the kinds of the mode, and with something written or
//! not), adds other alerts so the first ones leave the list, turns sound
//! off for a look, or relaunches the app from its saved file. In the
//! `changing` mix the way the wait is told can also change while it goes
//! on: its request shows a card or loses it, or its ask stops or starts
//! being listed (ticket 81).
//!
//! A wait is new when a look sees the session wait and the look before,
//! in the same run of the app, did not. Every new wait heard with sound
//! on is heard, but one rule of ticket 66: a card of a request decided
//! within the day before is that request (`CARD`), and is silent. No
//! other look plays anything: a wait that goes on, across sleeps, sound
//! off and on, alerts leaving the list and relaunches, sounds once. Where
//! its kind changes, a wait whose card was silent by that rule may sound
//! once at a later look with no card (`late`), as on the base; and a look
//! with no card right after a card of the request can ping again
//! (`after_card`): an extra ping, counted, not fixed (ticket 81).
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
    late: usize,
    after_card: usize,
    changes: usize,
    most_counts: usize,
    most_bytes: usize,
}

const LOOKS: usize = 150;

fn live(
    mode: &str,
    kinds: &[Kind],
    (writes, change): (bool, bool),
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
    // The wait's first look was silent by ticket 66's rule, and the look
    // before showed a card.
    let (mut quiet_start, mut carded) = (false, false);
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
        } else if change && waits && rng.below(100) < 30 {
            kind = kinds[rng.below(kinds.len() as u64) as usize];
            counts.changes += 1;
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
        let card = matches!(kind, Kind::Card | Kind::CardAsk | Kind::CardOnly);
        if waits && !was && !relaunched && sound {
            counts.new += 1;
            match (pings, card && decided) {
                (0, true) => counts.same_request += 1,
                (0, false) => counts.silent += 1,
                _ => counts.heard += 1,
            }
            quiet_start = pings == 0;
        } else if waits && was && !relaunched && quiet_start && pings == 1 {
            counts.late += 1;
            quiet_start = false;
        } else if waits && was && !relaunched && carded && !card {
            counts.after_card += pings;
        } else {
            counts.extra += pings;
        }
        quiet_start &= waits;
        carded = waits && card;
        (was, relaunched) = (waits, false);
        let held = lab.held();
        counts.most_counts = counts.most_counts.max(held.0);
        counts.most_bytes = counts.most_bytes.max(held.1);
    }
}

#[test]
fn every_new_wait_is_heard_and_a_wait_that_goes_on_sounds_once() {
    use Kind::*;
    const CHANGING: [Kind; 6] = [Named, Ask, Asked, Card, CardAsk, CardOnly];
    let modes: [(&str, &[Kind], (bool, bool)); 8] = [
        ("bare", &[Bare], (false, false)),
        ("named", &[Named], (false, false)),
        ("ask", &[Ask], (false, false)),
        ("mixed3", &NO_CARD, (false, false)),
        ("mixed3-writes", &NO_CARD, (true, false)),
        ("card", &[Card], (false, false)),
        ("mixed4", &[Bare, Named, Ask, Card], (false, false)),
        ("changing", &CHANGING, (false, true)),
    ];
    // The mixes run side by side: each is its own seeded lives.
    let folder = tempfile::tempdir().unwrap();
    let all: Vec<(&str, Counts)> = std::thread::scope(|scope| {
        let runs: Vec<_> = modes
            .iter()
            .map(|&(mode, kinds, how)| {
                let folder = folder.path();
                scope.spawn(move || {
                    let mut counts = Counts::default();
                    for seed in 1..=80 {
                        live(mode, kinds, how, seed, folder, &mut counts);
                    }
                    (mode, counts)
                })
            })
            .collect();
        runs.into_iter().map(|run| run.join().unwrap()).collect()
    });
    for (mode, counts) in all {
        println!("lives {mode}: {counts:?}");
        assert_eq!((counts.silent, counts.extra), (0, 0), "{mode}: {counts:?}");
        let changing = mode == "changing";
        let changed = counts.late + counts.after_card;
        assert!(changing || changed == 0, "{mode}: {counts:?}");
        assert!(!changing || counts.changes > 1000, "{mode}: {counts:?}");
        assert!(
            counts.heard > 0 && counts.most_counts <= 16,
            "{mode}: {counts:?}"
        );
    }
}
