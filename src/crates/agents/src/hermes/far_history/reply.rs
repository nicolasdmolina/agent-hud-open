//! What a reader answered, taken whole or not at all.
//!
//! An answer is lines of JSON, each an array that starts with a word:
//! `started` (or `no`, with why it did not start), then the rows, then
//! `end` with the status and the counts of what came before it. It is
//! used only when the last line is there, its counts match the rows, and
//! every row has the shape its kind has. Nothing of a rejected answer is
//! kept, and why it was rejected never holds a row's text.
use super::rows::{Batch, Cell, Far, No, Print, Prints, Rejected, Reply, Session};
use super::{Request, MOST_PAGE, MOST_USAGE_ROWS, SESSION_COLUMNS, USAGE_COLUMNS};
use serde_json::Value;
use std::collections::HashSet;

/// What the start of an answer says so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum First {
    /// No whole line yet.
    Pending,
    /// The reader passed its check and its deadline began.
    Started,
    Other,
}

pub fn first_line(stdout: &[u8]) -> First {
    match stdout.iter().position(|byte| *byte == b'\n') {
        None if stdout.len() > 256 => First::Other,
        None => First::Pending,
        Some(end) => match array(&stdout[..end]).as_deref() {
            Some([word, pid]) if word == "started" && pid.is_i64() => First::Started,
            _ => First::Other,
        },
    }
}

fn array(line: &[u8]) -> Option<Vec<Value>> {
    match serde_json::from_slice(line).ok()? {
        Value::Array(parts) if parts.first().is_some_and(Value::is_string) => Some(parts),
        _ => None,
    }
}

fn cells(parts: &[Value]) -> Option<Vec<Cell>> {
    parts.iter().map(Cell::of).collect()
}

/// The answer to `request` in `stdout`, whole, or why it is not used.
pub fn parse(request: &Request, stdout: &[u8]) -> Result<Reply, Rejected> {
    let mut lines: Vec<&[u8]> = stdout.split(|byte| *byte == b'\n').collect();
    // A whole answer ends with a line feed: what follows the last is empty.
    let cut = lines.pop().is_some_and(|rest| !rest.is_empty());
    let first = lines.first().and_then(|line| array(line));
    let first = first.as_deref().ok_or(Rejected::NotStarted)?;
    match first {
        [word, why] if word == "no" && lines.len() == 1 && !cut => {
            return Ok(Reply::No(match why.as_str() {
                Some("busy") => No::Busy,
                Some("sqlite") => No::Sqlite,
                Some("processes") => No::Processes,
                Some("request") => No::Request,
                _ => return Err(Rejected::Line(1)),
            }));
        }
        [word, pid] if word == "started" && pid.is_i64() => {}
        _ => return Err(Rejected::NotStarted),
    }
    if cut || lines.len() < 2 {
        return Err(Rejected::NoCompletion);
    }
    let end = array(lines[lines.len() - 1]).ok_or(Rejected::NoCompletion)?;
    let (status, counts) = match end.as_slice() {
        [word, status, counts @ ..] if word == "end" && counts.len() == 4 => {
            let counts: Option<Vec<i64>> = counts.iter().map(Value::as_i64).collect();
            (status.as_str(), counts)
        }
        _ => (None, None),
    };
    let (Some(status), Some(counts)) = (status, counts) else {
        return Err(Rejected::NoCompletion);
    };
    let body = &lines[1..lines.len() - 1];
    if status != "ok" {
        // No row comes with any other status.
        if let Some(line) = (!body.is_empty()).then_some(2) {
            return Err(Rejected::Line(line));
        }
        if counts != [0, 0, 0, 0] {
            return Err(Rejected::Counts);
        }
        let read = matches!(request, Request::Prints { .. } | Request::Sessions { .. });
        return match status {
            "none" if *request == Request::Held => Ok(Reply::Held(None)),
            "too-large" if read => Ok(Reply::TooLarge),
            "no-database" if read => Ok(Reply::Failed(Far::NoDatabase)),
            "db-open" if read => Ok(Reply::Failed(Far::Open)),
            "db-locked" if read => Ok(Reply::Failed(Far::Locked)),
            "db-schema" if read => Ok(Reply::Failed(Far::Schema)),
            "db-corrupt" if read => Ok(Reply::Failed(Far::Corrupt)),
            "db-error" if read => Ok(Reply::Failed(Far::Error)),
            _ => Err(Rejected::Status),
        };
    }
    let rows = body.iter().enumerate().map(|(index, line)| {
        // The first row is the answer's second line.
        array(line)
            .map(|parts| (index + 2, parts))
            .ok_or(Rejected::Line(index + 2))
    });
    let rows = rows.collect::<Result<Vec<_>, _>>()?;
    match request {
        Request::Probe if rows.is_empty() => Ok(Reply::Probed),
        Request::Probe => Err(Rejected::Line(2)),
        Request::Held => match rows.as_slice() {
            [(_, parts)] => match parts.as_slice() {
                [word, Value::String(text)] if word == "held" => {
                    Ok(Reply::Held(Some(text.clone())))
                }
                _ => Err(Rejected::Line(2)),
            },
            _ => Err(Rejected::Line(2)),
        },
        Request::Prints { after, limit, .. } => prints(&rows, *after, *limit, &counts),
        Request::Sessions { ids } => sessions(&rows, ids, &counts),
    }
}

fn prints(
    rows: &[(usize, Vec<Value>)],
    after: i64,
    limit: u32,
    counts: &[i64],
) -> Result<Reply, Rejected> {
    let page = limit.clamp(1, MOST_PAGE);
    let mut read = Prints::default();
    let mut last = after;
    for (line, parts) in rows {
        let print = match parts.as_slice() {
            [word, rowid, Value::String(id), Value::String(print)] if word == "p" => rowid
                .as_i64()
                .filter(|rowid| *rowid > last && !id.is_empty())
                .map(|rowid| Print {
                    rowid,
                    id: id.clone(),
                    print: print.clone(),
                }),
            _ => None,
        };
        let print = print.ok_or(Rejected::Line(*line))?;
        last = print.rowid;
        read.prints.push(print);
    }
    let count = read.prints.len() as i64;
    let seen = counts[2];
    if counts[0] != count
        || counts[1] != 0
        || seen < count
        || seen > i64::from(page)
        || counts[3] < last
    {
        return Err(Rejected::Counts);
    }
    read.seen = seen as u32;
    read.last = counts[3];
    read.ended = read.seen < page;
    Ok(Reply::Prints(read))
}

fn sessions(
    rows: &[(usize, Vec<Value>)],
    ids: &[String],
    counts: &[i64],
) -> Result<Reply, Rejected> {
    let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut told: HashSet<&str> = HashSet::new();
    let mut batch = Batch::default();
    // Whether the last line was a session's or one of its usage rows.
    let mut open = false;
    let mut usage = 0;
    for (line, parts) in rows {
        let wrong = Rejected::Line(*line);
        let mut named = |id: &str| asked.get(id).is_some_and(|id| told.insert(id));
        match parts.as_slice() {
            [word, rowid, Value::String(id), row @ .., Value::String(print)]
                if word == "s" && row.len() == SESSION_COLUMNS.len() =>
            {
                let (Some(rowid), Some(cells), true) = (rowid.as_i64(), cells(row), named(id))
                else {
                    return Err(wrong);
                };
                let (id, print) = (id.clone(), print.clone());
                batch.sessions.push(Session {
                    rowid,
                    id,
                    print,
                    cells,
                    usage: Vec::new(),
                });
                open = true;
            }
            [word, Value::String(id), row @ ..]
                if word == "u" && row.len() == USAGE_COLUMNS.len() =>
            {
                let session = batch
                    .sessions
                    .last_mut()
                    .filter(|session| open && session.id == *id);
                let (Some(session), Some(cells)) = (session, cells(row)) else {
                    return Err(wrong);
                };
                if session.usage.len() >= MOST_USAGE_ROWS {
                    return Err(wrong);
                }
                session.usage.push(cells);
                usage += 1;
            }
            [word, Value::String(id), why]
                if word == "skip" && why == "usage-rows" && named(id) =>
            {
                batch.skipped.push(id.clone());
                open = false;
            }
            _ => return Err(wrong),
        }
    }
    if counts[0] != batch.sessions.len() as i64 || counts[1] != usage {
        return Err(Rejected::Counts);
    }
    Ok(Reply::Sessions(batch))
}
