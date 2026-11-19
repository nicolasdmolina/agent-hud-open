//! One session of a Hermes `sessions` table as records: one for each of
//! its usage rows, or one from its own counts where it has none.
use super::{routed, rows::Sessions};
use crate::hermes::gateway::Profile;
use crate::transcript::{sql_column, sql_count, sql_model, sql_text, sql_time};
use crate::{
    transcript::records::{Record, Running},
    Usage,
};

/// The columns a usage row is read by, in the order they are read in.
const USAGE: [&str; 11] = [
    "model",
    "billing_provider",
    "billing_base_url",
    "billing_mode",
    "task",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "first_seen",
    "last_seen",
];

impl Sessions<'_> {
    /// Adds to `records` those of the session `row` holds: `id` as Agent
    /// HUD names it and `raw` as Hermes does, when it `started` and was
    /// last active (`updated`).
    pub(super) fn records(
        &self,
        row: &rusqlite::Row<'_>,
        (id, raw): (&str, &str),
        (started, updated): (i64, i64),
        records: &mut Vec<Record>,
    ) -> rusqlite::Result<()> {
        let label = self.mirror.as_ref().map(|mirror| mirror.label.clone());
        let live_total = super::rows::live_total(row)?;
        let profile = match &self.of {
            Profile::Named(name) => Some(name.as_str().to_owned()),
            Profile::Default => None,
        };
        let u = &self.usage;
        let title = sql_text(row, 3);
        let cwd = sql_text(row, 4);
        let title_fallback = sql_text(row, 5).map(|source| format!("Hermes · {source}"));
        // Where Hermes says the session was started: its stored `source`.
        let started_in = sql_text(row, 5)
            .map(|source| source.trim().to_owned())
            .filter(|source| !source.is_empty());
        // The profile the session names, else its database's (Open's
        // HermesSessions); the default profile names none.
        let client = sql_text(row, 16)
            .filter(|name| !name.is_empty() && name != "default")
            .or_else(|| profile.clone());
        let mut any = false;
        if u.contains("session_id") && u.contains("model") {
            let fields = USAGE.map(|key| sql_column(u, "u", key, "NULL"));
            let mut statement = self.db.prepare(&format!(
                "SELECT {} FROM session_model_usage u WHERE u.session_id=?1 LIMIT 501",
                fields.join(",")
            ))?;
            let mut usage = statement.query([raw])?;
            let mut limit = 0;
            while let Some(row) = usage.next()? {
                limit += 1;
                if limit > 500 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                any = true;
                let parts = (0..5)
                    .map(|i| sql_text(row, i).unwrap_or_default())
                    .collect::<Vec<_>>();
                let key = format!("usage:{id}:{}", serde_json::to_string(&parts).unwrap());
                records.push(Record {
                    id: id.to_owned(),
                    title: title.clone(),
                    title_fallback: title_fallback.clone(),
                    client: client.clone(),
                    machine: label.clone(),
                    live_total,
                    started_in: started_in.clone(),
                    cwd: cwd.clone(),
                    model: routed(
                        sql_model(sql_text(row, 0)),
                        sql_text(row, 1),
                        sql_text(row, 2),
                    ),
                    key,
                    at: sql_time(row, 9, true).unwrap_or(started),
                    usage: Usage {
                        input: sql_count(row, 5)?,
                        output: sql_count(row, 6)?,
                        read: sql_count(row, 7)?,
                        write: sql_count(row, 8)?,
                        reasoning: 0,
                    },
                    parent: None,
                    archived: Some(false),
                    updated: None,
                    duration_ms: None,
                    // Hermes adds to the row in place and stamps it.
                    running: Some(Running {
                        last_seen: sql_time(row, 10, true),
                    }),
                });
            }
        }
        if !any {
            let parts = (6..10)
                .map(|i| {
                    sql_text(row, i).unwrap_or_else(|| {
                        if i == 6 {
                            "unknown".into()
                        } else {
                            String::new()
                        }
                    })
                })
                .chain(Some(String::new()))
                .collect::<Vec<_>>();
            records.push(Record {
                id: id.to_owned(),
                title,
                title_fallback,
                client,
                machine: label.clone(),
                live_total,
                started_in: started_in.clone(),
                cwd,
                model: routed(
                    sql_model(sql_text(row, 6)),
                    sql_text(row, 7),
                    sql_text(row, 8),
                ),
                key: format!("usage:{}:{}", id, serde_json::to_string(&parts).unwrap()),
                at: started,
                usage: Usage {
                    input: sql_count(row, 10)?,
                    output: sql_count(row, 11)?,
                    read: sql_count(row, 12)?,
                    write: sql_count(row, 13)?,
                    reasoning: 0,
                },
                parent: None,
                archived: Some(false),
                updated: None,
                duration_ms: None,
                // Hermes adds to the session's own counts in place; its
                // activity is when they last grew.
                running: Some(Running {
                    last_seen: Some(updated),
                }),
            });
        }
        if let Some(record) = records.last_mut() {
            record.updated = Some(updated);
        }
        Ok(())
    }
}
