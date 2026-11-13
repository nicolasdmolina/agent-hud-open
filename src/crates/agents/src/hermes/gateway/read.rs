//! Reading one session in a poll, and saying what it waits on.
//!
//! The first `session.events.since` asks without an epoch, which Hermes
//! always answers empty with the stream's epoch and latest event. The
//! rest look for the oldest event Hermes still keeps, so that every
//! request whose event it keeps is seen. Hermes keeps the last 512 events
//! and 4 MiB of a stream. It answers a read whole if it starts no earlier
//! than the last event it dropped, and refuses it otherwise, without
//! saying where that is (`tui_gateway/event_replay.py`, `is_truncated`).
//! So the place is searched for: first where it is if only the count of
//! events made Hermes drop any, then by halving between what it refused
//! and what it answered.
//!
//! What was dropped cannot be read. Hermes lists a session as `waiting`
//! exactly while a question or an approval waits in it. A request asked
//! before the oldest event kept can be found only by a snapshot,
//! `session.resume`, which subscribes, so it is taken on a socket of its
//! own that is closed after the reply. One is taken the first time a
//! session is seen waiting with events dropped, or waiting with nothing
//! in its events to wait on, and again for as long as a card showing is
//! not accounted for by the events.
use super::effect::{Closed, Status};
use super::fold::{batches, fold, Reading};
use super::frame::Cursor;
use super::identity::SessionRef;
use super::poll::{Poll, Read, Shown, Step};
use serde_json::Value;
use std::collections::BTreeMap;

/// How many events Hermes keeps of one stream (`_REPLAY_BUFFER_MAX`): it
/// has dropped everything more than this before its latest event.
const RING: u64 = 512;
/// The most `session.events.since` calls one session gets in one poll:
/// one to learn where its stream is, and a search that halves 512 places.
const MAX_READS: u8 = 16;
/// How often the search tries again where the oldest event would be if
/// only the count made Hermes drop any, after the stream moved on.
const MAX_GUESSES: u8 = 3;

/// The search for the oldest event Hermes still keeps of one stream.
#[derive(Debug, Default)]
pub(super) struct Search {
    /// The `session.events.since` calls answered so far.
    reads: u8,
    guesses: u8,
    /// The latest place Hermes refused a read from: it has dropped the
    /// event after it.
    refused: Option<u64>,
    /// The earliest place Hermes answered a read from, and what the events
    /// after it say waits.
    best: Option<(u64, Reading)>,
    /// What the events read showed to be over, kept for a snapshot.
    pub(super) over: BTreeMap<String, Closed>,
}

impl Search {
    /// Where to read from next, in a stream whose latest event is `latest`.
    /// None when the oldest event kept is found, when nothing is kept, or
    /// when the session has had its calls.
    fn next(&mut self, latest: u64) -> Option<u64> {
        if self.reads >= MAX_READS {
            return None;
        }
        // Where the oldest event kept is if only the count of events made
        // Hermes drop any, and the earliest it can be by what was refused.
        let counted = latest.saturating_sub(RING);
        let floor = self.refused.map_or(counted, |at| counted.max(at + 1));
        match &self.best {
            Some((answered, _)) if floor >= *answered => None,
            Some((answered, _)) => Some(floor + (answered - floor) / 2),
            None if floor >= latest => (latest == 0 && self.refused.is_none()).then_some(0),
            // Not refused there yet: the first read, or the stream moved on.
            None if self.refused.is_none_or(|at| counted > at) && self.guesses < MAX_GUESSES => {
                self.guesses += 1;
                Some(counted)
            }
            // Hermes drops by size as well. Is anything kept at all?
            None => Some(latest - 1),
        }
    }
}

impl Poll {
    /// One answer to `session.events.since`.
    pub(super) fn read(
        &mut self,
        at: &SessionRef,
        from: Option<&Cursor>,
        mut search: Search,
        result: &Value,
    ) {
        let epoch = crate::text(&result["replay_epoch"]).or_else(|| crate::text(&result["epoch"]));
        let latest = result["last_sequence"]
            .as_u64()
            .or(result["latest_seq"].as_u64());
        let (Some(epoch), Some(latest)) = (epoch, latest) else {
            return self.set(at, Read::Failed);
        };
        search.reads += 1;
        let cut = result["snapshot_required"] == true || result["truncated"] == true;
        match from.filter(|from| from.epoch == epoch) {
            Some(from) if !cut => {
                let events = result["events"].as_array().map_or(&[][..], Vec::as_slice);
                let mut reading = fold(events);
                reading.asks.extend(batches(&result["open_requests"]));
                if search
                    .best
                    .as_ref()
                    .is_none_or(|(best, _)| from.seq < *best)
                {
                    search.best = Some((from.seq, reading));
                }
            }
            Some(from) => search.refused = search.refused.max(Some(from.seq)),
            // Not asked by its epoch yet, or replaced since: a new stream.
            None => {
                search = Search {
                    reads: search.reads,
                    ..Search::default()
                }
            }
        }
        if let Some(seq) = search.next(latest) {
            let cursor = Some(Cursor {
                epoch: epoch.to_owned(),
                seq,
            });
            let session = at.clone();
            let step = Step::Since { session, cursor };
            return self.set(at, Read::Wanted { step, search });
        }
        let (dropped, reading) = match search.best.take() {
            Some((from, reading)) => (from > 0, reading),
            // No event can be read. The sets of questions are told all
            // the same.
            None => {
                let asks = batches(&result["open_requests"]);
                let reading = Reading {
                    asks,
                    ..Reading::default()
                };
                (true, reading)
            }
        };
        self.settle(at, reading, dropped);
    }

    /// What the session waits on, from the events read; or a snapshot to
    /// take, where they cannot say. `dropped` is whether Hermes has dropped
    /// any event of the stream, so that a request may wait unseen.
    fn settle(&mut self, at: &SessionRef, mut reading: Reading, dropped: bool) {
        let Some(status) = self.sessions.get(at).map(|look| look.status) else {
            return;
        };
        let none = Shown::default();
        let shown = self.shown.get(at).unwrap_or(&none);
        if !matches!(status, Status::Working | Status::Waiting) {
            // No turn runs, so the turn of every card showing is over,
            // whether or not the events read still hold its end.
            for key in shown.prompts.iter().chain(&shown.sets) {
                reading.end(key, Closed::TurnEnded);
            }
        }
        // Once in each stretch of waiting: where events were dropped, a
        // request may wait that none of those kept tells of, whatever else
        // is seen waiting; and where nothing is seen waiting, something is.
        let first = !self.snapshotted.contains(at) && (dropped || !reading.prompts());
        // Whenever a card showing is not accounted for.
        let unexplained = |key: &String| !reading.holds(key) && !reading.over.contains_key(key);
        let read = if status == Status::Waiting && (first || shown.prompts.iter().any(unexplained))
        {
            let step = Step::Snapshot(at.clone());
            let over = reading.over;
            let search = Search {
                over,
                ..Search::default()
            };
            Read::Wanted { step, search }
        } else {
            Read::Done(reading)
        };
        self.set(at, read);
    }
}
