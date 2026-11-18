//! A mirror that has grown (ticket 54). Rows made from the live list
//! are never removed, and ticket 37 adds a far machine's whole stored
//! history, so a mirror holds thousands of rows. A session that begins
//! must still be listed at the first pass after its row's commit: core
//! reads the newest rows of a database first
//! (`agents::hermes::transcript`), where it used to reach them one pass
//! for every hundred rows before them. And that pass is the only one
//! the write costs: it looks through a mirror's old rows too.
use super::*;

/// The title of the session that begins, which none of the others has.
const NEW: &str = "Just begun on the far machine";

/// `count` made-up sessions, as a live list names them.
fn many(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|n| (format!("s{n:05}"), format!("Title {n}")))
        .collect()
}

impl Bench {
    /// A mirror of the default profile that holds `count` rows core has
    /// read. Every other one is as the far database gives it, set up by
    /// hand as ticket 37 will write it: durable, with tokens and a usage
    /// row. The rest are as the live list left them.
    fn holding(count: usize) -> (Self, Vec<(String, String)>) {
        let mut bench = bench("Mac mini");
        let held = many(count);
        bench.lists(&Profile::Default, &refs(&held), 1_000, 1);
        let db = rusqlite::Connection::open(bench.file(&Profile::Default)).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.execute_batch(
            "BEGIN;
             UPDATE sessions SET input_tokens=100, output_tokens=20, ended_at=1760000600
                WHERE rowid % 2 = 0;
             INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens,
                output_tokens, first_seen, last_seen)
                SELECT id, 'claude-test', 'anthropic', 100, 20, 1760000010, 1760000600
                FROM sessions WHERE rowid % 2 = 0;
             UPDATE hud_rows SET durable=1, print='print-1' WHERE session_id IN
                (SELECT id FROM sessions WHERE rowid % 2 = 0);
             COMMIT;",
        )
        .unwrap();
        drop(db);
        bench.turn();
        bench.reads_on();
        assert_eq!(bench.count(), count);
        (bench, held)
    }

    /// The passes the collector makes while core says logs wait, two
    /// seconds apart in the app, with nothing told and no file event:
    /// until none waits. How many those were.
    pub(super) fn reads_on(&mut self) -> usize {
        let mut passes = 0;
        while self.core.indexing() {
            self.silent_pass();
            passes += 1;
            assert!(passes < 1_000, "core never finished reading");
        }
        passes
    }

    pub(super) fn found(&mut self, search: &str) -> Value {
        let query = json!({"scope": "all", "search": search});
        self.core.request("sessions", query).unwrap()
    }

    /// How many sessions core lists. A page of the list is 200 at most.
    pub(super) fn count(&mut self) -> usize {
        self.found("")["total"].as_u64().unwrap() as usize
    }

    /// Whether the session titled `NEW` is listed.
    fn lists_the_new_one(&mut self) -> bool {
        let found = self.found(NEW);
        let items = found["items"].as_array().unwrap();
        items.iter().any(|item| item["title"] == NEW)
    }

    /// A session begins on the far machine: the next poll lists it with
    /// the rest. When its row was committed.
    fn begins(&mut self, held: &[(String, String)]) -> Instant {
        let mut listed = refs(held);
        listed.push(("znew", NEW));
        let tells = self.tells();
        self.lists(&Profile::Default, &listed, 2_000, 1);
        assert_eq!(self.tells(), tells + 1);
        *self.told.at.lock().unwrap().last().unwrap()
    }
}

fn refs(sessions: &[(String, String)]) -> Vec<(&str, &str)> {
    (sessions.iter())
        .map(|(id, title)| (id.as_str(), title.as_str()))
        .collect()
}

#[test]
fn a_new_row_in_a_large_mirror_is_listed_at_the_first_pass() {
    for count in [90, 250, 1_000, 5_000] {
        let (mut bench, held) = Bench::holding(count);
        assert!(!bench.lists_the_new_one());
        let committed = bench.begins(&held);
        assert_eq!(bench.turn(), 1);
        let listed = bench.lists_the_new_one();
        let took = committed.elapsed();
        assert!(listed, "{count} rows held: not listed at the first pass");
        assert!(took <= SOON, "{count} rows held: {took:?}");
        // That pass also looked through the old rows, which a write
        // begins anew: nothing is left for another pass, where it used
        // to take one pass for every hundred rows, two seconds apart.
        assert!(!bench.core.indexing(), "{count} rows held: logs wait");
        assert_eq!(bench.count(), count + 1);
        eprintln!(
            "large mirror: {count} rows held, the new row listed {took:?} after its commit, \
             at the first pass, with no pass after it"
        );
    }
}

/// What changes in a mirror that has grown shows after the one pass
/// its write asks for (ticket 60), and nothing waits for a pass more.
///
/// - A session's count: its own row is read again, the oldest in the
///   mirror as soon as the newest.
/// - The machine's label, which changes every row. A mirror's changed
///   rows are read in that one pass, where they took a pass for each
///   hundred, two seconds apart, and reached the newest rows last.
/// - And a session that begins after either is listed at the first
///   pass like any.
#[test]
fn what_changes_in_a_large_mirror_shows_after_one_pass() {
    use agent_hud_agents::hermes::gateway::Sample;
    for count in [1_000, 5_000] {
        let (mut bench, held) = Bench::holding(count);
        // The newest row and one of the oldest. Each has 120 tokens of
        // its own, and shows the count, which says more.
        for (round, at) in [count - 1, 7].into_iter().enumerate() {
            let (id, title) = &held[at];
            let total = 700 + round as u64;
            let sample = Sample {
                input: total,
                ..Sample::default()
            };
            let said = BTreeMap::from([(Profile::Default, vec![(id.clone(), sample)])]);
            let tells = bench.tells();
            bench.writer.counted(said, 3_000 + round as i64, 1);
            bench.writer.settle();
            until_told(&bench, tells + 1);
            let committed = *bench.told.at.lock().unwrap().last().unwrap();
            assert_eq!(bench.turn(), 1);
            // Said and not judged, like the label's below: among every
            // other test of a run the one pass can take longer than
            // the 2 s it is held to where it is timed, by hand
            // (`tests/far_rows_timing.rs`).
            let took = committed.elapsed();
            let found = bench.found(title);
            let row = found["items"].as_array().unwrap().iter();
            let row = row.clone().find(|item| item["title"] == title.as_str());
            assert_eq!(row.unwrap()["tokens"], json!(total), "{count} rows held");
            assert!(!bench.core.indexing(), "{count} rows held: logs wait");
            eprintln!("large mirror: {count} rows held, a count shown {took:?} after its commit");
        }

        bench.writer.relabel("Studio");
        bench.writer.settle();
        let committed = *bench.told.at.lock().unwrap().last().unwrap();
        assert_eq!(bench.turn(), 1);
        let studio = bench.found("studio")["total"].as_u64().unwrap();
        let took = committed.elapsed();
        assert_eq!(studio, count as u64, "{count} rows held");
        assert!(!bench.core.indexing(), "{count} rows held: logs wait");
        eprintln!("large mirror: {count} rows held, a new label on all {took:?} after its commit");

        let committed = bench.begins(&held);
        assert_eq!(bench.turn(), 1);
        let listed = bench.lists_the_new_one();
        let took = committed.elapsed();
        assert!(listed, "{count} rows held: not listed at the first pass");
        assert!(took <= SOON, "{count} rows held: {took:?}");
        assert!(!bench.core.indexing(), "{count} rows held: logs wait");
        assert_eq!(bench.count(), count + 1);
        let studio = bench.found("studio")["total"].as_u64().unwrap();
        assert_eq!(studio, count as u64 + 1);
    }
}

/// Waits until the writer has told core `count` times.
fn until_told(bench: &Bench, count: usize) {
    let limit = Instant::now() + Duration::from_secs(20);
    while bench.tells() < count {
        assert!(Instant::now() < limit, "{} tells", bench.tells());
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// And a mirror core has never read: the app was just launched, or its
/// ledger was made anew. The newest rows are the first read.
#[test]
fn a_new_row_is_listed_at_the_first_pass_of_a_mirror_never_read() {
    let mut bench = bench("Mac mini");
    let held = many(1_000);
    bench.lists(&Profile::Default, &refs(&held), 1_000, 1);
    let committed = bench.begins(&held);
    assert_eq!(bench.turn(), 1);
    let listed = bench.lists_the_new_one();
    let took = committed.elapsed();
    assert!(listed, "not listed at the first pass");
    assert!(took <= SOON, "{took:?}");
    assert!(bench.core.indexing(), "one pass read a thousand rows");
    bench.reads_on();
    assert_eq!(bench.count(), 1_001);
}
