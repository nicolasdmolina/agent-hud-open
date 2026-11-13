//! Reading one session in a poll, and saying what it waits on.
//!
//! Reading a session is `session.events.since` twice: once to learn the stream's epoch and
//! latest event, once for the events themselves. Hermes keeps the last 512
//! events and 4 MiB of a stream and refuses a read that starts before what
//! it keeps, so a stream that has dropped anything cannot be read from its
//! start. It is read from a little after the oldest event it can still
//! hold, and from nearer the end each time Hermes refuses.
//!
//! Hermes says a session `waiting` exactly while a question or an approval
//! waits in it. Where it says so and the events read show none waiting, or
//! do not account for one that is showing, the request was asked before the
//! events read (dropped from the stream, or asked in a stream since
//! replaced). Only a snapshot lists it: `session.resume`, which subscribes,
//! so it is taken on a socket of its own that is closed after the reply.
use super::effect::{Closed, Status};
use super::fold::{batches, fold};
use super::frame::Cursor;
use super::identity::SessionRef;
use super::poll::{Poll, Read, Shown, Step};
use serde_json::Value;
use std::collections::BTreeMap;

/// How many events Hermes keeps of one stream (`_REPLAY_BUFFER_MAX`). A
/// stream no longer than this may still hold its start.
const RING: u64 = 512;
/// How far before its latest event a stream is read from once it does not
/// hold its start. The first leaves room for the events that arrive
/// between learning the latest and reading; each next one is tried when
/// Hermes no longer keeps that much (it also drops by size).
const TAILS: [u64; 5] = [448, 256, 64, 16, 4];
/// The most `session.events.since` calls one session gets in one poll.
const MAX_READS: u8 = 7;

impl Poll {
    /// One answer to `session.events.since`. A read that names the stream's
    /// epoch and starts no earlier than what Hermes keeps comes back whole;
    /// any other comes back empty, saying where the stream is.
    pub(super) fn read(
        &mut self,
        at: &SessionRef,
        from: Option<&Cursor>,
        reads: u8,
        result: &Value,
    ) {
        let epoch = crate::text(&result["replay_epoch"]).or_else(|| crate::text(&result["epoch"]));
        let latest = result["last_sequence"]
            .as_u64()
            .or(result["latest_seq"].as_u64());
        let (Some(epoch), Some(latest)) = (epoch, latest) else {
            return self.set(at, Read::Failed);
        };
        let open = &result["open_requests"];
        let cut = result["snapshot_required"] == true || result["truncated"] == true;
        if from.is_some() && !cut {
            let events = result["events"].as_array().map_or(&[][..], Vec::as_slice);
            return self.settle(at, events, open);
        }
        // Refused in the stream asked: start nearer the end. A stream not
        // asked by its epoch yet, or replaced since, is read as a new one.
        let after = from.filter(|from| from.epoch == epoch).map(|from| from.seq);
        match tail(latest, after).filter(|_| reads + 1 < MAX_READS) {
            Some(seq) => self.set(
                at,
                Read::Wanted {
                    step: Step::Since {
                        session: at.clone(),
                        cursor: Some(Cursor {
                            epoch: epoch.to_owned(),
                            seq,
                        }),
                    },
                    reads: reads + 1,
                    over: BTreeMap::new(),
                },
            ),
            // Nothing of the stream can be read. What it says of the sets
            // of questions still holds.
            None => self.settle(at, &[], open),
        }
    }

    /// What the session waits on, from the events read; or a snapshot to
    /// take, where they cannot say.
    fn settle(&mut self, at: &SessionRef, events: &[Value], open: &Value) {
        let Some(status) = self.sessions.get(at).map(|look| look.status) else {
            return;
        };
        let mut reading = fold(events);
        reading.asks.extend(batches(open));
        let none = Shown::default();
        let shown = self.shown.get(at).unwrap_or(&none);
        if !matches!(status, Status::Working | Status::Waiting) {
            // No turn runs, so the turn of every card showing is over,
            // whether or not the events read still hold its end.
            for key in shown.prompts.iter().chain(&shown.sets) {
                reading.end(key, Closed::TurnEnded);
            }
        }
        let unexplained = |key: &String| !reading.holds(key) && !reading.over.contains_key(key);
        let unsure = !reading.prompts() || shown.prompts.iter().any(unexplained);
        let read = if status == Status::Waiting && unsure {
            Read::Wanted {
                step: Step::Snapshot(at.clone()),
                reads: 0,
                over: reading.over,
            }
        } else {
            Read::Done(reading)
        };
        self.set(at, read);
    }
}

/// Where to read a stream from, whose latest event is `latest`: its start
/// while it may still hold it, else a little after the oldest event it can
/// hold; and, once Hermes refused a read from `after`, the furthest back
/// that is nearer the end than that.
fn tail(latest: u64, after: Option<u64>) -> Option<u64> {
    let Some(after) = after else {
        return Some(if latest <= RING { 0 } else { latest - TAILS[0] });
    };
    TAILS
        .iter()
        .map(|back| latest.saturating_sub(*back))
        .find(|start| *start > after)
}
