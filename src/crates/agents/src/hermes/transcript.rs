//! Hermes SQLite session reader (`state.db`).
use crate::transcript::{sql_column, sql_columns, sql_count, sql_model, sql_text, sql_time};
use crate::{
    transcript::records::{Page, Record, Running},
    Usage,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

pub fn read(db: &Connection, path: &Path, cursor: &Value) -> rusqlite::Result<Page> {
    let s = sql_columns(db, "sessions")?;
    if !s.contains("id") || !s.contains("started_at") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let u = sql_columns(db, "session_model_usage")?;
    let fields = [
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
    ]
    .map(|key| sql_column(&s, "s", key, "NULL"));
    // A running session's activity: Hermes stamps each usage row's
    // `last_seen` as it adds to it (HermesSessions.swift:45-46).
    let last_seen = if u.contains("session_id") && u.contains("last_seen") {
        "(SELECT MAX(last_seen) FROM session_model_usage WHERE session_id=s.id)"
    } else {
        "NULL"
    };
    // What a session's tokens are now, to find one that grew without its
    // activity passing the watermark: a write stamped earlier than another
    // session's activity already read.
    let print = format!(
        "({})||'/'||{}",
        tokens(&s, "s"),
        if u.contains("session_id") {
            format!(
                "(SELECT COUNT(*)||'/'||TOTAL({}) FROM session_model_usage u WHERE u.session_id=s.id)",
                tokens(&u, "u")
            )
        } else {
            "''".into()
        }
    );
    let scan = cursor["scan"]
        .as_i64()
        .or_else(|| cursor.as_i64())
        .unwrap_or(0);
    let rowid_after = cursor["rowid_after"].as_i64().unwrap_or(0);
    let ts_after = cursor["ts_after"].as_i64().unwrap_or(0);
    let mut max_rowid = cursor["max_rowid"].as_i64().unwrap_or(rowid_after);
    let mut max_ts = cursor["max_ts"].as_i64().unwrap_or(ts_after);
    let prints = &cursor["prints"];
    let mut next = cursor["next"].as_object().cloned().unwrap_or_default();
    let mut statement = db.prepare(&format!(
        "SELECT s.rowid,s.id,s.started_at,{},{last_seen},{print} FROM sessions s WHERE s.rowid>?1 ORDER BY s.rowid LIMIT 100",
        fields.join(",")
    ))?;
    let mut rows = statement.query([scan])?;
    let mut records = Vec::new();
    let mut last = scan;
    let mut seen = 0;
    let folder = path.parent().unwrap_or(Path::new(""));
    let profile = folder
        .parent()
        .and_then(Path::file_name)
        .filter(|s| *s == "profiles")
        .and_then(|_| folder.file_name())
        .map(|s| s.to_string_lossy().into_owned());
    while let Some(row) = rows.next()? {
        seen += 1;
        last = row.get(0)?;
        max_rowid = max_rowid.max(last);
        let Some(raw) = sql_text(row, 1) else {
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
        max_ts = max_ts.max(updated);
        let print = Value::from(sql_text(row, 18).unwrap_or_default());
        let grew = prints[last.to_string()] != print;
        next.insert(last.to_string(), print);
        if last <= rowid_after && updated <= ts_after && !grew {
            continue;
        }
        let id = format!(
            "hermes:{}{raw}",
            profile
                .as_ref()
                .map(|p| format!("{p}:"))
                .unwrap_or_default()
        );
        let title = sql_text(row, 3);
        let cwd = sql_text(row, 4);
        let title_fallback = sql_text(row, 5).map(|source| format!("Hermes · {source}"));
        // The profile the session names, else its database's (Open's
        // HermesSessions); the default profile names none.
        let client = sql_text(row, 16)
            .filter(|name| !name.is_empty() && name != "default")
            .or_else(|| profile.clone());
        let mut any = false;
        if u.contains("session_id") && u.contains("model") {
            let fields = [
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
            ]
            .map(|key| sql_column(&u, "u", key, "NULL"));
            let mut statement = db.prepare(&format!(
                "SELECT {} FROM session_model_usage u WHERE u.session_id=?1 LIMIT 501",
                fields.join(",")
            ))?;
            let mut usage = statement.query([&raw])?;
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
                    id: id.clone(),
                    title: title.clone(),
                    title_fallback: title_fallback.clone(),
                    client: client.clone(),
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
                id: id.clone(),
                title,
                title_fallback,
                client,
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
    }
    Ok(Page {
        records,
        cursor: (seen == 100).then(|| {
            json!({
                "scan": last,
                "rowid_after": rowid_after,
                "ts_after": ts_after,
                "max_rowid": max_rowid,
                "max_ts": max_ts,
                "prints": prints,
                "next": next,
            })
        }),
        watermark: (seen < 100).then(|| {
            json!({
                "scan": 0,
                "rowid_after": max_rowid,
                "ts_after": max_ts,
                "prints": next,
            })
        }),
    })
}

/// The sum of a table's four token counts, each where it has the column.
fn tokens(columns: &std::collections::HashSet<String>, alias: &str) -> String {
    [
        "input_tokens",
        "output_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
    ]
    .map(|key| format!("COALESCE({},0)", sql_column(columns, alias, key, "0")))
    .join("+")
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
