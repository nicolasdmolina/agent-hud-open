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
//! before the oldest event read can be found only by a snapshot,
//! `session.resume`, which subscribes, so it is taken on a socket of its
//! own that is closed after the reply. One is wanted of a waiting session
//! whose stream has dropped events, whose search did not finish, whose
//! events hold nothing to wait on, or which shows a card its events do
//! not account for. It is taken unless one was taken of that session a
//! short while ago (`SNAPSHOT_EVERY_MS`); until the next, the cards the
//! events do not speak for stay as they are.
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
/// A stream that stands still needs at most 12; one that keeps moving can
/// use them all up, and the read is then known not to be whole.
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

/// What the search does next.
enum Next {
    /// Read from here.
    Read(u64),
    /// The oldest event kept is found, or nothing is kept: what was read
    /// is everything Hermes keeps.
    Found,
    /// The session has had its calls and the oldest event kept is not
    /// found: Hermes may keep events older than any read.
    Spent,
}

impl Search {
    /// Where to read from next, in a stream whose latest event is `latest`.
    fn next(&mut self, latest: u64) -> Next {
        // Where the oldest event kept is if only the count of events made
        // Hermes drop any, and the earliest it can be by what was refused.
        let counted = latest.saturating_sub(RING);
        let floor = self.refused.map_or(counted, |at| counted.max(at + 1));
        let from = match &self.best {
            Some((answered, _)) if floor >= *answered => return Next::Found,
            Some((answered, _)) => floor + (answered - floor) / 2,
            None if floor >= latest && (latest > 0 || self.refused.is_some()) => {
                return Next::Found
            }
            None if floor >= latest => 0,
            // Not refused there yet: the first read, or the stream moved on.
            None if self.refused.is_none_or(|at| counted > at) && self.guesses < MAX_GUESSES => {
                self.guesses += 1;
                counted
            }
            // Hermes drops by size as well. Is anything kept at all?
            None => latest - 1,
        };
        if self.reads >= MAX_READS {
            return Next::Spent;
        }
        Next::Read(from)
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
        let whole = match search.next(latest) {
            Next::Read(seq) => {
                let cursor = Some(Cursor {
                    epoch: epoch.to_owned(),
                    seq,
                });
                let session = at.clone();
                let step = Step::Since { session, cursor };
                return self.set(at, Read::Wanted { step, search });
            }
            Next::Found => true,
            Next::Spent => false,
        };
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
        self.settle(at, reading, dropped, whole);
    }

    /// What the session waits on, from the events read; or a snapshot to
    /// take, where they cannot say. `dropped` is whether Hermes has dropped
    /// any event of the stream, so that a request may wait unseen. `whole`
    /// is whether everything Hermes keeps was read.
    fn settle(&mut self, at: &SessionRef, mut reading: Reading, dropped: bool, whole: bool) {
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
        if status != Status::Waiting {
            // Hermes says no question or approval waits here. That, and
            // the sets of questions it lists, hold whatever was read.
            return self.set(at, Read::Done(reading));
        }
        // The cards showing that the events read do not speak for: all of
        // them where not everything kept was read, else those whose
        // request and whose end are both missing from the events.
        let silent =
            |key: &&String| !reading.holds(key) && (!whole || !reading.over.contains_key(*key));
        let unexplained: Vec<String> = shown.prompts.iter().filter(silent).cloned().collect();
        // A request may wait that no event read tells of: asked before
        // the oldest event kept, or before the oldest one read. And where
        // nothing is seen waiting, something is.
        let unsure = dropped || !whole || !reading.prompts() || !unexplained.is_empty();
        let read = if unsure && !self.recent.contains(at) {
            let step = Step::Snapshot(at.clone());
            let over = reading.over;
            let search = Search {
                over,
                ..Search::default()
            };
            Read::Wanted { step, search }
        } else {
            // A snapshot was taken of it not long ago. Until the next one,
            // the cards the events do not speak for stay as they are.
            reading.kept = unexplained;
            Read::Done(reading)
        };
        self.set(at, read);
    }
}
