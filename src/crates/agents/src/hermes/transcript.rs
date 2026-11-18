//! Hermes SQLite session reader (`state.db`).
//!
//! A page reads the newest sessions first, then goes on with the pass
//! over the old ones (ticket 54):
//!
//! - **New sessions**, those above the highest rowid read so far, newest
//!   first, a hundred a page at most. Where there were more than a page
//!   reads, the rest are a gap that later pages read, again from its
//!   newest end, once nothing newer waits. Each is read once.
//! - **The pass over the old ones**, the sessions the last pass ended
//!   with, from the lowest rowid up. It reads those whose counts grew or
//!   that were active since, as it always has: Hermes adds to a session
//!   in place (`session_counts`). A page looks at a hundred of them,
//!   and reads those of them that changed. In a mirror it looks at ten
//!   thousand, and reads as many as changed (ticket 60): a mirror's
//!   rows are short, since its writer cuts every word it stores, and
//!   every write to one begins this pass anew, which then ends in the
//!   page that began it and not a hundred rows a pass later, each pass
//!   at what a pass costs. So what changes in a mirror shows after one
//!   pass, whether it is one session's count or the label of them all.
//!
//! So a session that begins is read on the first page after it, however
//! many the database holds and wherever a pass has got to. Read at the
//! end of the pass, as it was, it waited one page for every hundred
//! sessions before it. A session's rowid tells the new from the old:
//! SQLite gives a new row a rowid above every one there is. A row put in
//! the place of the newest one, deleted just before, can get a rowid the
//! last pass ended with, and is then found by the pass like any old one.
use super::mirror;
use super::session_counts::SessionCounts;
use crate::transcript::records::Page;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

mod rows;
mod usage;
use rows::Sessions;

/// How many new sessions one page reads; and how many old ones it looks
/// at, and reads at most.
const PAGE: i64 = 100;
/// How many old sessions of a mirror one page looks at, and reads at
/// most.
const MIRROR_PAGE: i64 = 10_000;

pub fn read(db: &Connection, path: &Path, cursor: &Value) -> rusqlite::Result<Page> {
    // Checked on every page, before anything is read: a mirror that cannot
    // be trusted gives nothing.
    let mirror = mirror::identity(db)?;
    let sessions = Sessions::of(db, path, mirror)?;
    let whole = |key: &str| cursor[key].as_i64();
    let scan = whole("scan").or_else(|| cursor.as_i64()).unwrap_or(0);
    let rowid_after = whole("rowid_after").unwrap_or(0);
    let ts_after = whole("ts_after").unwrap_or(0);
    // The first page of a pass goes on from the last pass's watermark.
    let first = scan == 0 && cursor["max_rowid"].is_null();
    // The highest rowid read so far: what is above it is new.
    let mut top = whole("max_rowid").unwrap_or(rowid_after);
    let mut max_ts = whole("max_ts").unwrap_or(ts_after);
    let mut gaps = gaps(&cursor["gaps"]);
    let mut prints = SessionCounts::from_cursor(cursor);
    let mut records = Vec::new();

    // A new session is read whatever its counts, which are noted.
    let mut new = |rowid: i64, updated: i64, print: &str| {
        max_ts = max_ts.max(updated);
        prints.changed(rowid, print);
        true
    };
    let fresh = sessions.page(
        ("rowid>?1", true),
        &[top],
        (PAGE, PAGE),
        &mut new,
        &mut records,
    )?;
    if fresh.more {
        // More may lie between these and what was read before.
        gaps.push((top, fresh.lowest));
    }
    top = top.max(fresh.highest);
    let mut room = PAGE - fresh.seen;
    while let (Some((after, before)), true) = (gaps.last().copied(), room > 0) {
        let held = sessions.page(
            ("rowid>?1 AND rowid<?2", true),
            &[after, before],
            (room, room),
            &mut new,
            &mut records,
        )?;
        gaps.pop();
        if held.more {
            gaps.push((after, held.lowest));
        }
        room -= held.seen;
    }

    // The pass over the sessions the last one ended with: up to where it
    // has got to now, and whether that is their end.
    let (swept, ended) = if scan < rowid_after {
        let limits = match sessions.of_a_mirror() {
            true => (MIRROR_PAGE, MIRROR_PAGE),
            false => (PAGE, PAGE),
        };
        let old = sessions.page(
            ("rowid>?1 AND rowid<=?2", false),
            &[scan, rowid_after],
            limits,
            &mut |rowid, updated, print| {
                max_ts = max_ts.max(updated);
                let grew = prints.changed(rowid, print);
                grew || updated > ts_after
            },
            &mut records,
        )?;
        match old.more {
            true => (old.highest, false),
            false => (rowid_after, true),
        }
    } else {
        (scan, true)
    };
    let prints = prints.finish(scan, swept);
    let over = ended && gaps.is_empty();
    Ok(Page {
        records,
        watermark: if over {
            Some(json!({
                "scan": 0,
                "rowid_after": top,
                "ts_after": max_ts,
                "prints": prints,
            }))
        } else if first {
            // The pass's cursor now holds the map, so the last pass's
            // watermark is dropped rather than kept beside it; it is read only
            // once a pass ends, which leaves a new one.
            Some(Value::Null)
        } else {
            None
        },
        cursor: (!over).then(|| {
            json!({
                "scan": swept,
                "rowid_after": rowid_after,
                "ts_after": ts_after,
                "max_rowid": top,
                "max_ts": max_ts,
                "gaps": gaps,
                "prints": prints,
            })
        }),
    })
}

/// The stretches of new sessions a cursor says are still to be read:
/// each the rowids above its first number and below its second, the
/// newest stretch last.
fn gaps(cursor: &Value) -> Vec<(i64, i64)> {
    let stretch = |gap: &Value| Some((gap[0].as_i64()?, gap[1].as_i64()?));
    (cursor.as_array().into_iter().flatten())
        .filter_map(stretch)
        .collect()
}

/// A Hermes model with the route its billing provider and address name
/// (`endpoints::provider_route`), so a relay is not priced at a vendor's list.
fn routed(model: String, provider: Option<String>, base_url: Option<String>) -> String {
    match crate::endpoints::provider_route(provider.as_deref(), base_url.as_deref()) {
        Some(route) => format!("{model}#{route}"),
        None => model,
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
