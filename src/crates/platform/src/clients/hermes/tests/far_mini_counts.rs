//! What the two sources of a far session's count cost on a real far
//! machine, for the test that follows one and only reads
//! (`far_mini.rs`, with `AGENT_HUD_FAR_COUNTS` set). Ticket 35 chose
//! between them from these numbers and the lab's.
//!
//! For each live session a gateway lists, at most `MOST` of them, on
//! one socket opened as the polls' is and subscribed to nothing:
//!
//! - `session.stats`, the call the polls make for a session that works:
//!   how long it took, how large the reply was, the four counts;
//! - `session.events.since` as a poll's read begins, once for where the
//!   stream is and once for its last `TAIL` events: how long, how
//!   large, and how many of those events are `session.usage`.
//!
//! What is said of a session is its place in the list, where Hermes
//! says it was started, what it is doing and numbers. No id, no title
//! and no word of any event is printed or kept.
use super::super::{opening, Shared};
use agent_hud_agents::hermes::gateway::{events_since, stats, Connection, Cursor, Scope};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The most sessions of one gateway that are asked.
const MOST: usize = 12;
/// How many of a stream's last events are read.
const TAIL: u64 = 128;
const WAIT: Duration = Duration::from_secs(15);
const FOUR: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
];

fn bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |text| text.len())
}

fn four(value: &Value) -> Vec<Option<u64>> {
    FOUR.iter().map(|key| value[*key].as_u64()).collect()
}

fn summed(name: &str, mut values: Vec<u128>) -> String {
    values.sort();
    match values.as_slice() {
        [] => format!("{name}: none"),
        all => format!(
            "{name}: n={} least={} middle={} most={}",
            all.len(),
            all[0],
            all[all.len() / 2],
            all[all.len() - 1]
        ),
    }
}

/// Asks both sources of every live session `shared`'s gateway lists,
/// and says what each cost.
pub(super) fn measure(say: &mut dyn FnMut(String), shared: &Arc<Shared>) {
    let reader = shared.reader.clone();
    let (instance, sessions) = {
        let model = shared.model();
        let Connection::Ready(instance) = model.gateway.connection() else {
            return say(format!("COUNTS {reader}: the gateway is not ready"));
        };
        let listing = model.gateway.listing();
        let statuses = model.gateway.statuses();
        let mut sessions = Vec::new();
        for (profile, rows) in listing.map(|listing| listing.profiles).unwrap_or_default() {
            for (id, row) in rows {
                let Some(at) = model.gateway.session(&profile, &id) else {
                    continue;
                };
                let status = statuses.iter().find(|(of, _)| *of == at);
                let status = status.map_or("?".to_owned(), |(_, status)| format!("{status:?}"));
                sessions.push((at, row.source.unwrap_or_else(|| "?".into()), status));
            }
        }
        (instance.clone(), sessions)
    };
    let version = instance.version.clone().unwrap_or_else(|| "?".into());
    let listed = sessions.len();
    say(format!(
        "COUNTS {reader}: Hermes {version}, {listed} live sessions listed, asking {}",
        listed.min(MOST)
    ));
    let mut socket = match opening::opened(shared, &instance) {
        Ok(socket) => socket,
        Err(_) => return say(format!("COUNTS {reader}: no socket opened")),
    };
    let (mut stats_ms, mut stats_bytes, mut events_ms, mut events_bytes) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut answered, mut counting, mut with_usage, mut shape) = (0, 0, 0, None);
    for (place, (at, source, status)) in sessions.iter().take(MOST).enumerate() {
        // The count, as the polls ask it.
        let began = Instant::now();
        let said = socket.call(&stats(at, Scope::Host), WAIT);
        let took = began.elapsed().as_millis();
        let stats_line = match &said {
            Ok(result) => {
                answered += 1;
                let counts = four(result);
                counting += usize::from(counts.iter().flatten().any(|count| *count > 0));
                stats_ms.push(took);
                stats_bytes.push(bytes(result) as u128);
                shape.get_or_insert_with(|| keys(result));
                format!("stats ms={took} bytes={} four={counts:?}", bytes(result))
            }
            Err(failure) => format!("stats FAILED after {took} ms: {failure:?}"),
        };
        // The events, as a poll's read of the session begins.
        let began = Instant::now();
        let place_of = socket.call(&events_since(at, None, Scope::Host), WAIT);
        let mut size = place_of.as_ref().map_or(0, bytes);
        let mut events_line = "events: the stream was not told".to_owned();
        if let Ok(told) = &place_of {
            let epoch = told["replay_epoch"].as_str().or(told["epoch"].as_str());
            let latest = told["last_sequence"]
                .as_u64()
                .or(told["latest_seq"].as_u64());
            if let (Some(epoch), Some(latest)) = (epoch, latest) {
                let cursor = Cursor {
                    epoch: epoch.to_owned(),
                    seq: latest.saturating_sub(TAIL),
                };
                let read = socket.call(&events_since(at, Some(&cursor), Scope::Host), WAIT);
                let took = began.elapsed().as_millis();
                size += read.as_ref().map_or(0, bytes);
                let none = Vec::new();
                let events = read.as_ref().ok();
                let events = events
                    .and_then(|read| read["events"].as_array())
                    .unwrap_or(&none);
                let usage: Vec<&Value> = (events.iter())
                    .filter(|event| event["type"] == "session.usage")
                    .collect();
                with_usage += usize::from(!usage.is_empty());
                let newest = usage.last().map(|event| four(&event["payload"]));
                events_ms.push(took);
                events_bytes.push(size as u128);
                events_line = format!(
                    "events calls=2 ms={took} bytes={size} latest_seq={latest} read={} usage_events={} newest_usage_four={newest:?}",
                    events.len(),
                    usage.len()
                );
            }
        }
        say(format!(
            "COUNTS {reader} #{place} source={source} status={status}: {stats_line} | {events_line}"
        ));
    }
    socket.close();
    let asked = listed.min(MOST);
    say(format!(
        "COUNTS {reader} SUMMARY: session.stats answered {answered} of {asked}, {counting} with a count over zero; \
         {with_usage} of {asked} had a session.usage event among their last {TAIL} events"
    ));
    say(format!("COUNTS {reader} {}", summed("stats ms", stats_ms)));
    say(format!(
        "COUNTS {reader} {}",
        summed("stats bytes", stats_bytes)
    ));
    say(format!(
        "COUNTS {reader} {}",
        summed("events ms (2 calls)", events_ms)
    ));
    say(format!(
        "COUNTS {reader} {}",
        summed("events bytes (2 calls)", events_bytes)
    ));
    if let Some(shape) = shape {
        say(format!("COUNTS {reader} the stats reply's fields: {shape}"));
    }
}

/// The names of a reply's fields, each with the kind of its value.
fn keys(result: &Value) -> String {
    let kind = |value: &Value| match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    };
    let fields = result.as_object().into_iter().flatten();
    let named: Vec<String> = fields
        .map(|(key, value)| format!("{key}:{}", kind(value)))
        .collect();
    named.join(" ")
}
