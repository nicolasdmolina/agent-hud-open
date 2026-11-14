//! A takeover's sightings against the cards already heard (ticket 41, after
//! its third verifier). A card the transcript already names, by its `callId`
//! or the one possible pairing, stands for its own call and for no other
//! sighting; only a card no call names may stand for one unpaired sighting
//! of its kind. The verifier's probes (`verifier-3/tracker-probes.rs`).
use super::*;

/// A row of `session` with `cards` question cards and `sightings` question
/// sightings; the first `named` cards name calls `call-0`.., which the
/// first sightings are. `transcript` says the row came from the log.
fn row(session: &str, cards: usize, sightings: usize, named: usize, transcript: bool) -> Value {
    let ids: Vec<_> = (0..cards).map(|n| format!("card-{n}")).collect();
    let pairs: Vec<_> = ids.iter().map(|id| (id.as_str(), "question")).collect();
    let mut row = waiting(session, &pairs);
    if transcript {
        row["logPath"] = json!(format!("synthetic/{session}.jsonl"));
    }
    row["heard"] = json!((0..sightings)
        .map(|n| json!({"id":format!("log:claude:{session}:call-{n}"),"kind":"question"}))
        .collect::<Vec<_>>());
    if cards == 0 {
        row["liveFacts"]["requests"] = row["heard"].clone();
    }
    row["calls"] = json!((0..named)
        .map(|n| (format!("card-{n}"), json!(format!("call-{n}"))))
        .collect::<serde_json::Map<String, Value>>());
    row
}

#[test]
fn a_call_matched_card_stands_for_no_second_sighting() {
    let mut tracker = started();
    assert_eq!(
        look(
            &mut tracker,
            &no_events(),
            &[row("s", 1, 0, 1, false)],
            &on(),
            T + 1
        )
        .len(),
        1
    );
    // The card's own call-0 and a new call-1 come with the log's row.
    let alerts = look(
        &mut tracker,
        &no_events(),
        &[row("s", 1, 2, 1, true)],
        &on(),
        T + 2,
    );
    assert_eq!(
        alerts,
        [needs_you("s")],
        "the card already is call-0; call-1 is a new question"
    );
}

#[test]
fn named_and_unnamed_cards_against_any_count_of_sightings() {
    let mut wrong = Vec::new();
    for cards in 1..=3 {
        for sightings in 0..=6 {
            for named in 0..=cards.min(sightings) {
                let mut tracker = started();
                look(
                    &mut tracker,
                    &no_events(),
                    &[row("s", cards, 0, named, false)],
                    &on(),
                    T + 1,
                );
                let got = look(
                    &mut tracker,
                    &no_events(),
                    &[row("s", cards, sightings, named, true)],
                    &on(),
                    T + 2,
                )
                .len();
                // Sightings beyond the cards are new questions: one sound.
                let expected = usize::from(sightings > cards);
                if got != expected {
                    wrong.push(format!(
                        "cards={cards} sightings={sightings} named={named}: {got}"
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A row of session `s` whose `cards` are (id, kind, call) and whose
/// `sightings` are (call, kind).
fn cards_row(
    cards: &[(&str, &str, Option<&str>)],
    sightings: &[(&str, &str)],
    transcript: bool,
) -> Value {
    let shown: Vec<_> = cards.iter().map(|(id, kind, _)| (*id, *kind)).collect();
    let mut row = waiting("s", &shown);
    if transcript {
        row["logPath"] = json!("synthetic/s.jsonl");
    }
    row["heard"] = json!(sightings
        .iter()
        .map(|(call, kind)| json!({"id":format!("log:claude:s:{call}"),"kind":kind}))
        .collect::<Vec<_>>());
    if cards.is_empty() {
        row["liveFacts"]["requests"] = row["heard"].clone();
    }
    row["calls"] = json!(cards
        .iter()
        .filter_map(|(id, _, call)| call.map(|call| (id.to_string(), json!(call))))
        .collect::<serde_json::Map<_, _>>());
    row
}

fn take(tracker: &mut Tracker, row: Value, at: i64) -> usize {
    look(tracker, &no_events(), &[row], &on(), T + at).len()
}

/// A card a call once named stays that call when a later view of it names
/// none. The fourth verifier's
/// `a_previously_named_card_cannot_regain_credit_when_its_current_call_metadata_is_absent`.
#[test]
fn a_card_once_named_never_stands_for_another_sighting() {
    let mut tracker = started();
    take(
        &mut tracker,
        cards_row(&[("card", "question", Some("a"))], &[], false),
        1,
    );
    let unnamed = [("card", "question", None)];
    let got = take(
        &mut tracker,
        cards_row(&unnamed, &[("a", "question"), ("new", "question")], true),
        2,
    );
    assert_eq!(got, 1, "the card is call a; new is a new question");
}

/// A card the one possible pairing made a call at one takeover is that call
/// at the next. The fourth verifier's
/// `a_second_takeover_must_not_reuse_an_already_inferred_cards_allowance`.
#[test]
fn a_card_paired_at_one_takeover_stands_for_nothing_at_the_next() {
    let mut tracker = started();
    let cards = [
        ("named", "question", Some("a")),
        ("unnamed", "question", None),
    ];
    assert_eq!(take(&mut tracker, cards_row(&cards, &[], false), 1), 1);
    let first = cards_row(&cards, &[("a", "question"), ("b", "question")], true);
    assert_eq!(take(&mut tracker, first, 2), 0);
    assert_eq!(take(&mut tracker, cards_row(&cards, &[], false), 3), 0);
    let second = cards_row(
        &cards,
        &[("a", "question"), ("b", "question"), ("new", "question")],
        true,
    );
    assert_eq!(
        take(&mut tracker, second, 4),
        1,
        "the unnamed card was b; new is a new question"
    );
}
