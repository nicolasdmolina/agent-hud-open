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
//! within the day before is that request (`CARD`), and is silent. Over
//! whole waits: one with sound on and the app running at every look of it
//! is heard at some look (`never`), but a wait that began as such a card.
//! A wait already heard plays nothing more, across sleeps, sound off and
//! on, alerts leaving the list, relaunches and kind changes, but a look
//! with no card right after a card of the request (`after_card`: an extra
//! ping, counted, not changed). A wait not heard yet may sound at a later
//! look (`owed`), as on the base.
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
    /// Whole waits with sound on and the app running at every look of
    /// them that no look of them was heard, but ticket 66's cards.
    never: usize,
    /// The same, for a wait that began as a card of a request decided
    /// within the day (ticket 66): silent by design, as on the base.
    never_by_ticket_66: usize,
    /// A ping in a wait no look of which had been heard yet.
    owed: usize,
    /// A ping in a wait already heard, after a relaunch or a look with
    /// sound off inside it (a wait listed again late after a relaunch
    /// sounds again: ticket 75).
    not_whole: usize,
    /// A ping in a wait already heard, at a look with no card right after
    /// a card of the request (ticket 81, measured, not changed).
    after_card: usize,
    /// A ping in a wait already heard where the look names another request
    /// than the one before (a flag alone, another card): a new request in
    /// the wait sounds (ticket 66).
    other_request: usize,
    /// Any other ping in a wait already heard, or with no wait.
    extra: usize,
    changes: usize,
    most_counts: usize,
    most_bytes: usize,
}

const LOOKS: usize = 150;

/// One wait as it goes: whether a look of it was heard, whether every look
/// had sound on and the app running, and whether it began as ticket 66's
/// card.
#[derive(Default)]
struct Going {
    heard: bool,
    whole: bool,
    by_ticket_66: bool,
}

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
    let (mut going, mut carded): (Option<Going>, bool) = (None, false);
    let mut before = kind;
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
        let card_name = if kind == Kind::Other { OTHER } else { CARD };
        let decided = lab
            .tracker
            .decided
            .get(card_name)
            .is_some_and(|t| at - t < DAY);
        let pings = lab.see(gap, Some(row), kind, others, sound);
        let card = matches!(
            kind,
            Kind::Card | Kind::CardAsk | Kind::CardOnly | Kind::Other
        );
        let audible = sound && !relaunched;
        if !waits || !was {
            if let Some(ended) = going.take() {
                end(ended, counts);
            }
        }
        if waits && !was {
            let by_ticket_66 = card && decided;
            if audible {
                counts.new += 1;
                match (pings, by_ticket_66) {
                    (0, true) => counts.same_request += 1,
                    (0, false) => counts.silent += 1,
                    _ => counts.heard += 1,
                }
            }
            let (heard, whole) = (pings > 0, audible);
            going = Some(Going {
                heard,
                whole,
                by_ticket_66,
            });
        } else if let Some(wait) = going.as_mut().filter(|_| waits) {
            wait.whole &= audible;
            let other = kind != before && (unrelated(kind) || unrelated(before));
            match (wait.heard, other, carded && !card) {
                (false, ..) => counts.owed += pings,
                _ if !wait.whole => counts.not_whole += pings,
                (true, true, _) => counts.other_request += pings,
                (true, false, true) => counts.after_card += pings,
                (true, false, false) => counts.extra += pings,
            }
            wait.heard |= pings > 0;
        } else {
            counts.extra += pings;
        }
        (carded, before) = (waits && card, kind);
        (was, relaunched) = (waits, false);
        let held = lab.held();
        counts.most_counts = counts.most_counts.max(held.0);
        counts.most_bytes = counts.most_bytes.max(held.1);
    }
    if let Some(ended) = going {
        end(ended, counts);
    }
}

/// A way of telling the wait that names no request, or another one.
fn unrelated(kind: Kind) -> bool {
    matches!(kind, Kind::Bare | Kind::Other)
}

/// A wait is over: one heard at no look, with sound on and the app running
/// at all of them, is counted.
fn end(wait: Going, counts: &mut Counts) {
    match (wait.heard || !wait.whole, wait.by_ticket_66) {
        (true, _) => {}
        (false, true) => counts.never_by_ticket_66 += 1,
        (false, false) => counts.never += 1,
    }
}

#[test]
fn every_new_wait_is_heard_and_a_wait_that_goes_on_sounds_once() {
    use Kind::*;
    const CHANGING: [Kind; 8] = [Named, Ask, Asked, Card, CardAsk, CardOnly, Bare, Other];
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
        let wanted = (0, 0, 0);
        let got = (counts.silent, counts.never, counts.extra);
        assert_eq!(got, wanted, "{mode}: {counts:?}");
        let changing = mode == "changing";
        let changed = counts.owed + counts.after_card + counts.other_request + counts.not_whole;
        assert!(changing || changed == 0, "{mode}: {counts:?}");
        assert!(!changing || counts.changes > 1000, "{mode}: {counts:?}");
        assert!(
            counts.heard > 0 && counts.most_counts <= 16,
            "{mode}: {counts:?}"
        );
    }
}
