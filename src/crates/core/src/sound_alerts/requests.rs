//! Which requests a session waits on, each by every name it goes by, read
//! from what was there before cards hid any transcript sightings (`Heard`).
//! Names are joined only on evidence: a card and a sighting naming the same
//! tool call are one request; a card naming no call and a sighting are one
//! request only while both are there at the same look and that is the only
//! pairing possible. Anything else is a request of its own, preferring an
//! occasional extra sound to a silenced question.
use serde_json::Value;
use std::collections::HashMap;

/// What the snapshot's rows no longer show that the decision needs.
#[derive(Default)]
pub(crate) struct Heard<'a> {
    /// Carded sessions' transcript requests before cards hid them.
    pub sightings: Option<&'a crate::presentation::requests::Heard>,
    /// The tool call each waiting card holds, by card id, where named.
    pub calls: HashMap<String, String>,
}
pub(super) struct Request {
    pub names: Vec<String>,
    /// Its kind ("permission", "question", "plan"), as the row names it.
    pub kind: String,
}
struct Row<'a> {
    kind: &'a Value,
    call: Option<String>,
    names: Vec<String>,
}

fn row<'a>(id: &str, kind: &'a Value, call: Option<String>) -> Row<'a> {
    let names = [
        Some(id.to_owned()),
        call.as_ref().map(|c| format!("call:{c}")),
    ];
    Row {
        kind,
        call,
        names: names.into_iter().flatten().collect(),
    }
}

pub(super) fn requests(session: &Value, source: &str, id: &str, heard: &Heard) -> Vec<Request> {
    let shown = session["liveFacts"]["requests"].as_array();
    // What cards hid counts only while the row as finally presented still
    // waits on something: settling a session whose process has gone clears
    // its requests, and with them what was heard before.
    let raw = heard
        .sightings
        .filter(|_| shown.is_some_and(|rows| !rows.is_empty()))
        .and_then(|all| all.get(&format!("{source}:{id}")));
    let prefix = format!("log:{source}:{id}:");
    let mut sightings: Vec<Row> = raw
        .or(shown)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let rid = r["id"].as_str()?;
            let call = rid.strip_prefix(&prefix).map(str::to_owned);
            rid.starts_with("log:").then(|| row(rid, &r["kind"], call))
        })
        .collect();
    let mut cards: Vec<Row> = shown
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let rid = r["id"].as_str().filter(|rid| !rid.starts_with("log:"))?;
            Some(row(rid, &r["kind"], heard.calls.get(rid).cloned()))
        })
        .collect();
    for card in &mut cards {
        let Some(call) = &card.call else { continue };
        if let Some(i) = sightings.iter().position(|s| s.call.as_ref() == Some(call)) {
            card.names.extend(sightings.remove(i).names);
        }
    }
    let kinds: Vec<Value> = cards.iter().map(|card| card.kind.clone()).collect();
    for kind in &kinds {
        let unnamed: Vec<usize> = (0..cards.len())
            .filter(|&c| cards[c].call.is_none() && cards[c].kind == kind)
            .collect();
        let seen: Vec<usize> = (0..sightings.len())
            .filter(|&s| sightings[s].kind == kind)
            .collect();
        if let ([c], [s]) = (unnamed.as_slice(), seen.as_slice()) {
            let names = sightings.remove(*s).names;
            cards[*c].names.extend(names);
        }
    }
    cards
        .into_iter()
        .chain(sightings)
        .map(|r| Request {
            names: r.names,
            kind: r.kind.as_str().unwrap_or("").to_owned(),
        })
        .collect()
}
