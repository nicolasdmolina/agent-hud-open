//! Counts a client adds to in place, a running total per row (Hermes's
//! `session_model_usage`), as WholeFileStore.swift's `SessionLedger.totals`
//! keeps them: the counts a row had when first read stay at the row's own
//! time, and what it gains later is dated when it is seen, one event per
//! 15-minute period. The log holds both, and which rows it has seen, so a
//! read that finds nothing new adds nothing and a later read or a relaunch
//! never counts a row twice.
//!
//! A running total only grows: each count is cumulative. A smaller total
//! (a read caught mid-write, or a correction) adds nothing and takes nothing
//! back; later growth counts only once it passes the most already counted.
use super::{records::Running, Log, Usage};
use serde_json::{json, Value};

/// The period growth is grouped by, as Open's ledger buckets it.
const PERIOD_MS: i64 = 15 * 60 * 1000;

/// The rows a log has seen, by key, with the latest `last_seen` each had
/// (null when the client gave none); a row read at zero is seen without a
/// usage event.
const SEEN: &str = "runningSeen";

impl Log {
    /// Counts `total`, the row's counts so far, under `key`: in whole at `at`
    /// on first sight, after that only what it gained. Growth is dated at the
    /// row's `last_seen` when that moved past the stamp the row had when last
    /// read, otherwise at `now`, the time of this read: a stale or missing
    /// stamp does not say when the growth happened. Growth keys extend `key`
    /// with `@<period>`, so no other key may start with `key@`.
    #[allow(clippy::too_many_arguments)]
    pub fn count_running_total(
        &mut self,
        key: String,
        model: &str,
        at: i64,
        running: Running,
        now: i64,
        total: Usage,
        duration_ms: Option<u64>,
    ) {
        let before = self.source_state[SEEN].get(&key).cloned();
        let sighted = before.is_some() || self.messages.contains_key(&key);
        let before = before.as_ref().and_then(Value::as_i64);
        let latest = match (before, running.last_seen) {
            (Some(old), Some(new)) => Some(old.max(new)),
            (old, new) => old.or(new),
        };
        self.source_state[SEEN][key.as_str()] = json!(latest);
        if !sighted {
            self.count_timed_source(key, model, at, total, false, duration_ms);
            return;
        }
        let mut held = self.messages.get(&key).copied().unwrap_or_default();
        let prefix = format!("{key}@");
        for (other, usage) in &self.messages {
            if other.starts_with(&prefix) {
                held.add(*usage);
            }
        }
        let gained = Usage {
            input: total.input.saturating_sub(held.input),
            output: total.output.saturating_sub(held.output),
            read: total.read.saturating_sub(held.read),
            write: total.write.saturating_sub(held.write),
            reasoning: total.reasoning.saturating_sub(held.reasoning),
        };
        if gained.total() == 0 {
            return;
        }
        let grown_at = match (running.last_seen, before) {
            (Some(stamp), Some(before)) if stamp > before => stamp.min(now),
            _ => now,
        };
        let growth_key = format!("{prefix}{}", grown_at.div_euclid(PERIOD_MS) * PERIOD_MS);
        let mut growth = self.messages.get(&growth_key).copied().unwrap_or_default();
        growth.add(gained);
        self.count_timed_source(growth_key, model, grown_at, growth, false, None);
    }

    /// Forgets which running-total rows the log has seen, with its usage.
    pub(super) fn forget_running_totals(&mut self) {
        if let Some(state) = self.source_state.as_object_mut() {
            state.remove(SEEN);
        }
    }
}
