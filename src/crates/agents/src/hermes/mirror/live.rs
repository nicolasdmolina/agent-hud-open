//! What a far session has used so far, as its gateway last said
//! (`gateway::Sample`), kept in a mirror's `hud_live` for showing while
//! the session runs. It is no usage: the reader puts it on a record as
//! `live_total`, which is shown and never metered, and the ledger's only
//! input for a far session stays the rows its own database gave.
//!
//! This is the one place that writes `hud_live`, and it writes nothing
//! else: a sample never touches a row the ledger reads, and what is
//! read from a far database never touches a sample, so neither can undo
//! the other whichever is written last.
//!
//! A sample is dropped where it is all zeros (what Hermes answers while
//! it holds no agent for the session), where it totals no more than the
//! one kept, and where it was read no later than the one kept. So the
//! count kept never goes down, and the same sample twice changes
//! nothing. One of a session the mirror has no row for is not kept.
use crate::hermes::gateway::Sample;
use rusqlite::{params, Connection};

/// Keeps each of `samples` (a session's id as Hermes has it, its counts
/// and when they were read) that says more than the mirror holds; how
/// many it kept.
pub fn live(db: &Connection, samples: &[(String, Sample, i64)]) -> rusqlite::Result<usize> {
    let total = |of: &str| {
        format!("{of}input_tokens+{of}output_tokens+{of}cache_read_tokens+{of}cache_write_tokens")
    };
    let mut keep = db.prepare(&format!(
        "INSERT INTO hud_live (session_id, input_tokens, output_tokens, cache_read_tokens,
            cache_write_tokens, read_ms)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS (SELECT 1 FROM sessions WHERE id=?1)
         ON CONFLICT(session_id) DO UPDATE SET input_tokens=excluded.input_tokens,
            output_tokens=excluded.output_tokens, cache_read_tokens=excluded.cache_read_tokens,
            cache_write_tokens=excluded.cache_write_tokens, read_ms=excluded.read_ms
         WHERE excluded.read_ms > hud_live.read_ms AND {} > {}",
        total("excluded."),
        total("hud_live."),
    ))?;
    // A count past what a column holds is kept as the most it holds.
    let held = |count: u64| i64::try_from(count).unwrap_or(i64::MAX);
    let mut kept = 0;
    for (id, sample, read_ms) in samples {
        if sample.total() == 0 {
            continue;
        }
        kept += keep.execute(params![
            id,
            held(sample.input),
            held(sample.output),
            held(sample.cache_read),
            held(sample.cache_write),
            read_ms,
        ])?;
    }
    Ok(kept)
}
