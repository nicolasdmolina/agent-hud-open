//! A count that moves and a label that changes in a mirror that has
//! grown (ticket 60). Each write to a mirror asks core for one pass,
//! and what the write changed shows after that pass: nothing waits for
//! a pass more, however many rows the mirror holds and wherever in it
//! the session is. Before ticket 54 a count took a pass for each
//! hundred rows (11 at 1,000); a label still did, and reached the
//! newest rows last.
//!
//! The mirror here holds a thousand rows. The reader's own tests hold
//! the same of five thousand (`agents`, `transcript_tests.rs`), and the
//! timing lab measures both (`platform`, `far_rows_timing.rs`).
use super::*;

const ROWS: usize = 1_000;

/// A mirror of `ROWS` sessions from the live list beside `s1`, and a
/// core that has read them all.
fn grown() -> (Far, AgentHudCore) {
    let far = far();
    far.db.execute_batch("BEGIN").unwrap();
    for at in 0..ROWS {
        far.lists(&format!("t{at:05}"));
    }
    far.db.execute_batch("COMMIT").unwrap();
    let mut core = far.open();
    core.request("refresh", json!({})).unwrap();
    let mut passes = 1;
    while !core.reads.pending.is_empty() {
        core.request("snapshot", json!({})).unwrap();
        passes += 1;
        assert!(passes < 100, "the mirror is never read");
    }
    let far_rows = |head: &&LogHead| head.machine.is_some();
    assert_eq!(core.logs.values().filter(far_rows).count(), ROWS + 1);
    (far, core)
}

/// The one pass a write asks for. Whether core says more waits after it.
fn pass(core: &mut AgentHudCore) -> bool {
    core.request("refresh", json!({})).unwrap();
    !core.reads.pending.is_empty()
}

fn head<'a>(core: &'a AgentHudCore, session: &str) -> &'a LogHead {
    let ends = format!(":{session}");
    let found = core.logs.values().find(|head| head.id.ends_with(&ends));
    found.unwrap_or_else(|| panic!("no log for {session}"))
}

/// What the session's own page shows it has used: the list gives a
/// page of its rows, and this session may be on any.
fn shown(core: &mut AgentHudCore, session: &str) -> Value {
    let id = format!("hermes:{KEY}/default:{session}");
    let asked = json!({"id": id, "source": "hermes"});
    let detail = core.request("session_detail", asked).unwrap();
    detail["session"]["tokens"].clone()
}

#[test]
fn a_count_that_moves_in_a_grown_mirror_shows_after_one_pass() {
    let (far, mut core) = grown();
    // The newest row, one in the middle and the oldest.
    for (at, which) in [ROWS - 1, ROWS / 2, 0].into_iter().enumerate() {
        let session = format!("t{which:05}");
        assert_eq!(head(&core, &session).live_total, None);
        let total = 500 + at as u64;
        assert_eq!(far.sample(&session, total, 5), 1);
        assert!(!pass(&mut core), "a pass more waits after {session}");
        assert_eq!(head(&core, &session).live_total, Some(total));
        assert_eq!(shown(&mut core, &session), json!(total));
    }
    // And it moves again: one pass again.
    let newest = format!("t{:05}", ROWS - 1);
    assert_eq!(far.sample(&newest, 900, 9), 1);
    assert!(!pass(&mut core));
    assert_eq!(shown(&mut core, &newest), json!(900));
    // Nothing of it was counted.
    assert_eq!(ledger(&core), (0, 0));
}

#[test]
fn a_new_label_in_a_grown_mirror_shows_on_every_row_after_one_pass() {
    let (far, mut core) = grown();
    let key = machine_key(
        Some("0123456789abcdef0123456789abcdef"),
        "/Users/USER/.hermes",
    )
    .unwrap();
    let labelled = |core: &AgentHudCore, label: &str| {
        let shows = |head: &&LogHead| head.machine.as_deref() == Some(label);
        core.logs.values().filter(shows).count()
    };
    assert_eq!(labelled(&core, "Mac mini"), ROWS + 1);
    assert!(written::relabel(&far.db, &key, "Studio").unwrap());
    assert!(!pass(&mut core), "a pass more waits after the label");
    assert_eq!(labelled(&core, "Studio"), ROWS + 1);
    // A count that moves just after shows after its own one pass.
    let newest = format!("t{:05}", ROWS - 1);
    assert_eq!(far.sample(&newest, 700, 5), 1);
    assert!(!pass(&mut core));
    assert_eq!(shown(&mut core, &newest), json!(700));
    assert_eq!(head(&core, &newest).machine.as_deref(), Some("Studio"));
}
