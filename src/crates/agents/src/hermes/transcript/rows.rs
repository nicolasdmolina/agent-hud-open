//! Rows of a Hermes `sessions` table as records: one stretch of the table
//! at a time, in the order and within the bounds the page asks for
//! (`super::read`). A session's usage rows are read only once the page
//! wants the session.
use crate::hermes::gateway::{hud_session_id, stored_session_id, Machine, Profile};
use crate::hermes::mirror::{self, Mirror};
use crate::hermes::session_counts;
use crate::transcript::records::Record;
use crate::transcript::{sql_column, sql_columns, sql_text, sql_time};
use rusqlite::Connection;
use std::cell::Cell;
use std::time::{Duration, Instant};
use std::{collections::HashSet, path::Path};

/// The columns a session is read by after its rowid, id and start, in
/// the order they are read in.
const SESSION: [&str; 14] = [
    "title",
    "cwd",
    "source",
    "model",
    "billing_provider",
    "billing_base_url",
    "billing_mode",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "ended_at",
    "last_activity_at",
    "profile_name",
];

/// The sessions of one database, and whose they are.
pub(super) struct Sessions<'a> {
    pub(super) db: &'a Connection,
    /// `SELECT … FROM sessions s`: the columns a session is read by.
    select: String,
    /// The columns `session_model_usage` has.
    pub(super) usage: HashSet<String>,
    pub(super) mirror: Option<Mirror>,
    pub(super) of: Profile,
    /// Whether its sessions' replies can be asked for (`replied`).
    replies: bool,
    /// How long this read's questions about replies took so far.
    asked: Cell<Duration>,
}

/// The most time a read's questions about replies may take together, all
/// pages of it: past it a session's reply is not asked for and is unknown.
const REPLY_BUDGET: Duration = Duration::from_millis(50);

/// The rows one stretch held.
#[derive(Clone, Copy, Default)]
pub(super) struct Span {
    /// How many rows there were, read or not.
    pub seen: i64,
    /// The lowest and the highest rowid among them.
    pub lowest: i64,
    pub highest: i64,
    /// The stretch stopped at one of its limits: there may be more.
    pub more: bool,
}

impl<'a> Sessions<'a> {
    /// The sessions of `db` at `path`, a mirror's where `mirror` says so.
    pub(super) fn of(
        db: &'a Connection,
        path: &Path,
        mirror: Option<Mirror>,
    ) -> rusqlite::Result<Self> {
        let s = sql_columns(db, "sessions")?;
        if !s.contains("id") || !s.contains("started_at") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let u = sql_columns(db, "session_model_usage")?;
        let fields = SESSION.map(|key| sql_column(&s, "s", key, "NULL"));
        // A running session's activity: Hermes stamps each usage row's
        // `last_seen` as it adds to it (HermesSessions.swift:45-46).
        let last_seen = if u.contains("session_id") && u.contains("last_seen") {
            "(SELECT MAX(last_seen) FROM session_model_usage WHERE session_id=s.id)"
        } else {
            "NULL"
        };
        let print = session_counts::sql(&s, &u);
        let live = mirror::live_sql(db, mirror.as_ref())?;
        // Whether its sessions' replies can be asked for: a stored assistant
        // message, in this machine's database. A mirror holds no messages.
        let m = sql_columns(db, "messages").unwrap_or_default();
        let replies = mirror.is_none() && m.contains("session_id") && m.contains("role");
        // A `state.db` under a Hermes home is this machine's, and its folder
        // says whose: a named profile's in `profiles/<name>`, else the default
        // profile's of whatever root holds it. A mirror's own table says whose
        // it is, whatever folder holds it.
        let of = match &mirror {
            Some(mirror) => mirror.profile.clone(),
            None => Profile::of_home(path.parent().unwrap_or(Path::new(""))),
        };
        Ok(Self {
            db,
            select: format!(
                "SELECT s.rowid,s.id,s.started_at,{},{last_seen},{print},{live} FROM sessions s",
                fields.join(",")
            ),
            usage: u,
            mirror,
            of,
            replies,
            asked: Cell::new(Duration::ZERO),
        })
    }

    /// Whether these are a mirror's sessions.
    pub(super) fn of_a_mirror(&self) -> bool {
        self.mirror.is_some()
    }

    /// Whether the session Hermes calls `raw` holds a reply of the model: a
    /// stored assistant message. None where its database cannot say, the
    /// question fails, or the read's questions have used `REPLY_BUDGET`: a
    /// question is cut off when the budget runs out (SQLite's progress
    /// handler, set for it alone), so all of a read's questions together add
    /// at most about that much to it, and none fails it.
    fn replied(&self, raw: &str) -> Option<bool> {
        let left = REPLY_BUDGET.saturating_sub(self.asked.get());
        if !self.replies || left.is_zero() {
            return None;
        }
        let started = Instant::now();
        let deadline = started + left;
        let cut = move || Instant::now() >= deadline;
        self.db.progress_handler(1_000, Some(cut)).ok()?;
        let sql = "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND role='assistant')";
        let found = (self.db.prepare_cached(sql))
            .and_then(|mut question| question.query_row([raw], |row| row.get::<_, i64>(0)));
        let _ = self.db.progress_handler(0, None::<fn() -> bool>);
        self.asked.set(self.asked.get() + started.elapsed());
        found.ok().map(|found| found != 0)
    }

    /// The rows whose rowid is in `range`, a condition over the numbers
    /// `bounds`: the highest of them where `newest` and else the lowest,
    /// in the table's order either way. At most `looked` rows are looked
    /// at, and the stretch stops once `read` sessions were read.
    /// `wanted` is told each session's rowid, when it was last active and
    /// its counts, and says whether the session is read: only then are
    /// its usage rows read, and its records added to `records`.
    pub(super) fn page(
        &self,
        (range, newest): (&str, bool),
        bounds: &[i64],
        (looked, read): (i64, i64),
        wanted: &mut dyn FnMut(i64, i64, &str) -> bool,
        records: &mut Vec<Record>,
    ) -> rusqlite::Result<Span> {
        let limit = bounds.len() + 1;
        let sql = match newest {
            true => format!(
                "{} WHERE s.rowid IN (SELECT rowid FROM sessions WHERE {range} ORDER BY rowid DESC LIMIT ?{limit}) ORDER BY s.rowid",
                self.select
            ),
            false => format!(
                "{} WHERE {} ORDER BY s.rowid LIMIT ?{limit}",
                self.select,
                range.replace("rowid", "s.rowid")
            ),
        };
        let mut statement = self.db.prepare(&sql)?;
        let limits = bounds.iter().chain(Some(&looked));
        let mut rows = statement.query(rusqlite::params_from_iter(limits))?;
        let label = self.mirror.as_ref().map(|mirror| mirror.label.clone());
        let mut span = Span::default();
        let mut taken = 0;
        while taken < read {
            let Some(row) = rows.next()? else {
                break;
            };
            let rowid: i64 = row.get(0)?;
            span.lowest = if span.seen == 0 {
                rowid
            } else {
                span.lowest.min(rowid)
            };
            span.highest = span.highest.max(rowid);
            span.seen += 1;
            let Some(raw) = sql_text(row, 1) else {
                continue;
            };
            // A row whose id can be no session's is left out, like one without.
            let id = match &self.mirror {
                Some(mirror) => stored_session_id(&mirror.key, &self.of, &raw),
                None => hud_session_id(&Machine::This, &self.of, &raw),
            };
            let Some(id) = id else {
                continue;
            };
            let Some(started) = sql_time(row, 2, true) else {
                continue;
            };
            let updated = sql_time(row, 14, true)
                .into_iter()
                .chain(sql_time(row, 15, true))
                .chain(sql_time(row, 17, true))
                .chain(Some(started))
                .max()
                .unwrap_or(started);
            // A reply stored since the last read reads the session again.
            let counts = sql_text(row, 18).unwrap_or_default();
            // Only a session that shows no tokens is asked of its reply.
            let counted = (10..14).any(|i| row.get::<_, Option<i64>>(i).ok().flatten() > Some(0));
            let replied = (!counted && live_total(row)?.is_none_or(|n| n == 0))
                .then(|| self.replied(&raw))
                .flatten();
            let counts = match replied {
                Some(found) => format!("{counts}/{found}"),
                None => counts,
            };
            let print = mirror::print(label.as_deref(), counts, live_total(row)?);
            if !wanted(rowid, updated, &print) {
                continue;
            }
            taken += 1;
            let whose = (id.as_str(), raw.as_str());
            self.records(row, whose, (started, updated), replied, records)?;
        }
        span.more = span.seen == looked || taken == read;
        Ok(span)
    }
}

/// What a mirror's `hud_live` says the session `row` holds has used so
/// far (`mirror::live_sql`); None where it says nothing.
pub(super) fn live_total(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<u64>> {
    let total = row.get::<_, Option<i64>>(19)?;
    Ok(total.map(|total| total.max(0) as u64))
}
