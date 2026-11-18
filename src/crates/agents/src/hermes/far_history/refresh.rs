//! What the refresh of a far machine's history decides, apart from doing
//! it (ticket 37; the platform's Hermes channel runs the reads and has
//! the clock). Three kinds of work wait in every tick:
//!
//! 1. the *first queue*: sessions that are live, just ended, or held by
//!    the desktop app, read by id;
//! 2. the *recent sweep*: fingerprints of the sessions active in the last
//!    days, a page at a time;
//! 3. the *whole sweep*: fingerprints of the whole table, a page at a
//!    time by rowid, beginning anew at its end. Only it finds an old
//!    session whose counts changed with no change to its stamps.
//!
//! **Fairness.** The first queue does not get everything: every fourth
//! read of a tick is the whole sweep's, however full the first queue is,
//! so the sweep advances in every tick. And a whole sweep must finish
//! within a set time: once the last one that finished is older than that,
//! or none ever did, the sweep goes first until it is done.
use super::usable_id;
use serde_json::Value;
use std::{collections::HashSet, time::Duration};

/// One read in this many is the whole sweep's.
pub const SHARE: u32 = 4;
/// The most sessions taken from one `active_sessions.json`.
const MOST_HELD: usize = 500;

/// Whose a read of a tick is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    First,
    Recent,
    Sweep,
    /// Nothing more to do in this tick.
    Rest,
}

/// Whose the `read`th read of a tick is, counted from one. `overdue`:
/// the last whole sweep that finished is older than it may be. `first`
/// and `recent`: that kind of work waits. `swept`: the whole sweep had a
/// read in this tick already.
pub fn turn(read: u32, overdue: bool, first: bool, recent: bool, swept: bool) -> Turn {
    if overdue || read.is_multiple_of(SHARE) {
        Turn::Sweep
    } else if first {
        Turn::First
    } else if recent {
        Turn::Recent
    } else if !swept {
        // A tick with nothing else to do still moves the sweep on.
        Turn::Sweep
    } else {
        Turn::Rest
    }
}

/// Whether a whole sweep is overdue at `now_ms`: none ever finished, or
/// the last one did longer ago than `within`.
pub fn overdue(done_ms: Option<i64>, now_ms: i64, within: Duration) -> bool {
    let within = i64::try_from(within.as_millis()).unwrap_or(i64::MAX);
    done_ms.is_none_or(|done| now_ms.saturating_sub(done) > within)
}

/// The reads of a tick that are the whole sweep's at least, of `reads`
/// in all, while the first queue is full and no sweep is overdue.
pub fn sweep_reads(reads: u32) -> u32 {
    reads / SHARE
}

/// The pages one whole sweep of a table of `rows` rows reads, `page`
/// rows a page: a page that is full does not say the table ended, so a
/// table that ends on one takes one page more.
pub fn sweep_pages(rows: u64, page: u32) -> u64 {
    rows / u64::from(page.max(1)) + 1
}

/// The sessions the far `runtime/active_sessions.json` says the desktop
/// app holds, by Hermes's id. The text is the far machine's and is not
/// trusted: anything that is not an entry with `surface` `desktop` and an
/// id that can be asked for is left out, and no more than a bounded
/// number are taken.
pub fn desktop_held(text: &str) -> Vec<String> {
    let Ok(data) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let entries = match &data {
        Value::Object(map) => map.get("entries"),
        other => Some(other),
    };
    let mut seen = HashSet::new();
    let held = |entry: &Value| {
        let id = entry.get("session_id")?.as_str()?;
        let desktop = entry.get("surface")?.as_str()? == "desktop";
        (desktop && usable_id(id)).then(|| id.to_owned())
    };
    (entries.and_then(Value::as_array).into_iter().flatten())
        .filter_map(held)
        .filter(|id| seen.insert(id.clone()))
        .take(MOST_HELD)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whose each read of a tick of `reads` is, with the first queue
    /// never empty.
    fn full(reads: u32, overdue: bool) -> Vec<Turn> {
        (1..=reads)
            .map(|read| turn(read, overdue, true, true, false))
            .collect()
    }

    #[test]
    fn one_read_in_four_is_the_whole_sweeps_however_full_the_first_queue_is() {
        use Turn::{First, Sweep};
        assert_eq!(
            full(8, false),
            [First, First, First, Sweep, First, First, First, Sweep]
        );
        for reads in [4, 7, 8, 12, 40] {
            let swept = full(reads, false)
                .iter()
                .filter(|turn| **turn == Sweep)
                .count();
            assert_eq!(swept as u32, sweep_reads(reads));
            assert!(swept >= 1, "the sweep advances in a tick of {reads}");
        }
    }

    #[test]
    fn an_overdue_sweep_goes_first_until_it_is_done() {
        assert_eq!(full(6, true), [Turn::Sweep; 6]);
        // Done in the tick's third read: the rest is as in any tick.
        let rest: Vec<Turn> = (4..=8)
            .map(|read| turn(read, false, true, true, true))
            .collect();
        use Turn::{First, Sweep};
        assert_eq!(rest, [Sweep, First, First, First, Sweep]);
    }

    #[test]
    fn the_recent_sweep_goes_once_the_first_queue_is_empty_and_then_the_tick_rests() {
        assert_eq!(turn(1, false, false, true, false), Turn::Recent);
        // Nothing else waits: the whole sweep still gets its one read.
        assert_eq!(turn(1, false, false, false, false), Turn::Sweep);
        assert_eq!(turn(2, false, false, false, true), Turn::Rest);
        // And its share comes round whatever else is done.
        assert_eq!(turn(4, false, false, false, true), Turn::Sweep);
    }

    #[test]
    fn a_sweep_is_overdue_when_none_finished_or_the_last_is_older_than_the_set_time() {
        let day = Duration::from_secs(24 * 3600);
        let now = 1_760_000_000_000;
        assert!(overdue(None, now, day));
        assert!(!overdue(Some(now - 86_400_000), now, day));
        assert!(overdue(Some(now - 86_400_001), now, day));
        // A clock that went back makes nothing overdue.
        assert!(!overdue(Some(now + 5_000), now, day));
    }

    #[test]
    fn a_whole_sweep_reads_one_page_past_a_table_that_ends_on_a_full_one() {
        assert_eq!(sweep_pages(0, 500), 1);
        assert_eq!(sweep_pages(499, 500), 1);
        assert_eq!(sweep_pages(500, 500), 2);
        assert_eq!(sweep_pages(1990, 500), 4);
    }

    #[test]
    fn only_desktop_sessions_with_an_id_that_can_be_asked_for_are_held() {
        let text = r#"{"entries": [
            {"session_id": "d-one", "surface": "desktop", "pid": 1},
            {"session_id": "t-one", "surface": "telegram"},
            {"session_id": "d-one", "surface": "desktop"},
            {"session_id": "d-two", "surface": "desktop"},
            {"session_id": "", "surface": "desktop"},
            {"session_id": 7, "surface": "desktop"},
            {"session_id": "bad\u0007id", "surface": "desktop"},
            {"surface": "desktop"}, "text", null]}"#;
        assert_eq!(desktop_held(text), ["d-one", "d-two"]);
        // The older shape, a bare list.
        assert_eq!(
            desktop_held(r#"[{"session_id": "d-three", "surface": "desktop"}]"#),
            ["d-three"]
        );
        for none in ["", "not json", "{}", "7", r#"{"entries": {"a": 1}}"#] {
            assert!(desktop_held(none).is_empty(), "{none}");
        }
        let many: Vec<Value> = (0..MOST_HELD + 50)
            .map(|n| serde_json::json!({"session_id": format!("d-{n}"), "surface": "desktop"}))
            .collect();
        let many = serde_json::json!({ "entries": many }).to_string();
        assert_eq!(desktop_held(&many).len(), MOST_HELD);
    }
}
