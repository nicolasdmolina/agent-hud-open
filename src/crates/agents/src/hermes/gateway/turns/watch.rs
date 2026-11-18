//! What one poll learns of the streams of a far machine's sessions, for
//! its turns (`turns.rs`): the looks at the sessions it does not read,
//! the reads that follow a look where a stream has moved, and what the
//! reads it makes anyway for the cards say of the turns.
use super::super::fold::Reading;
use super::super::frame::{event, Cursor};
use super::super::identity::SessionRef;
use super::super::poll::Step;
use super::super::read::{Next, Search};
use serde_json::Value;
use std::collections::BTreeMap;

/// What one poll learns of the streams.
#[derive(Debug)]
pub(in super::super) struct Watch {
    /// Where each stream was seen when the poll began.
    pub(super) before: BTreeMap<SessionRef, Cursor>,
    pub(super) streams: BTreeMap<SessionRef, Stream>,
}

/// One session's stream as one poll saw it.
#[derive(Debug, Default)]
pub(super) struct Stream {
    /// The call still to be answered, of a session the poll does not read.
    wanted: Option<(Step, Search)>,
    pub(super) epoch: Option<String>,
    /// The stream was replaced while the poll ran.
    pub(super) replaced: bool,
    /// Where the stream was when it first answered.
    pub(super) looked: Option<u64>,
    /// The latest event a read of this poll reached.
    pub(super) through: Option<u64>,
    /// The turn events read, by their place: the turn, and whether it ended.
    pub(super) events: BTreeMap<u64, (u64, bool)>,
}

/// Where a stream is, by one answer to `session.events.since`: its epoch,
/// its latest event, and whether the read was refused.
fn place(result: &Value) -> Option<(&str, u64, bool)> {
    let epoch = crate::text(&result["replay_epoch"]).or_else(|| crate::text(&result["epoch"]))?;
    let latest = (result["last_sequence"].as_u64()).or(result["latest_seq"].as_u64())?;
    let cut = result["snapshot_required"] == true || result["truncated"] == true;
    Some((epoch, latest, cut))
}

impl Stream {
    /// Takes in one answer: where the stream is, and the turn events of a
    /// read that was answered from `from`.
    fn note(&mut self, from: Option<&Cursor>, result: &Value) {
        let Some((epoch, latest, cut)) = place(result) else {
            return;
        };
        if self.epoch.as_deref() != Some(epoch) {
            *self = Self {
                wanted: self.wanted.take(),
                replaced: self.epoch.is_some(),
                epoch: Some(epoch.to_owned()),
                looked: Some(latest),
                ..Self::default()
            };
        }
        if cut || from.is_none_or(|from| from.epoch != epoch) {
            return;
        }
        let events = result["events"].as_array().map_or(&[][..], Vec::as_slice);
        for seen in events.iter().filter_map(event) {
            let ended = match seen.kind {
                "message.start" => false,
                "message.complete" => true,
                _ => continue,
            };
            let at = seen.at.filter(|at| at.epoch == epoch);
            if let (Some(at), Some(turn)) = (at, seen.generation) {
                self.events.insert(at.seq, (turn, ended));
            }
        }
        self.through = self.through.max(Some(latest));
    }
}

impl Watch {
    /// A session the poll lists and does not read is looked at.
    pub(in super::super) fn look(&mut self, at: &SessionRef) {
        let (session, cursor) = (at.clone(), None);
        let step = Step::Watch { session, cursor };
        let stream = self.streams.entry(at.clone()).or_default();
        stream.wanted = Some((step, Search::default()));
    }

    pub(in super::super) fn steps(&self) -> impl Iterator<Item = Step> + '_ {
        let wanted = |stream: &Stream| Some(stream.wanted.as_ref()?.0.clone());
        self.streams.values().filter_map(wanted)
    }

    /// The poll read `at` for its cards: what that read says of its turns
    /// is taken from the same answer, at no call more.
    pub(in super::super) fn read(
        &mut self,
        at: &SessionRef,
        from: Option<&Cursor>,
        result: &Value,
    ) {
        self.streams
            .entry(at.clone())
            .or_default()
            .note(from, result);
    }

    pub(in super::super) fn failed(&mut self, step: &Step) {
        if let Step::Watch { session, .. } = step {
            if let Some(stream) = self.streams.get_mut(session) {
                stream.wanted.take_if(|(asked, _)| asked == step);
            }
        }
    }

    /// One answer to a look, or to a read that follows one. A stream that
    /// has not moved past its place, was never seen, or is another one
    /// than was seen, is read no further. One that moved is read on from
    /// its place; where Hermes refuses that, having dropped the events
    /// after it, the oldest event it still keeps is searched for as a
    /// poll searches (`read.rs`), never below the place.
    pub(in super::super) fn answer(&mut self, step: &Step, result: &Value) {
        let Step::Watch { session, cursor } = step else {
            return;
        };
        let Some(stream) = self.streams.get_mut(session) else {
            return;
        };
        let Some((_, mut search)) = stream.wanted.take_if(|(asked, _)| asked == step) else {
            return;
        };
        let Some((epoch, latest, cut)) = place(result) else {
            return;
        };
        stream.note(cursor.as_ref(), result);
        search.reads += 1;
        let Some(before) = self.before.get(session).filter(|at| at.epoch == epoch) else {
            return;
        };
        let from = match cursor.as_ref().filter(|from| from.epoch == epoch) {
            None if latest > before.seq => Some(before.seq),
            None => None,
            // Everything after the place was read.
            Some(from) if !cut && from.seq <= before.seq => None,
            Some(from) => {
                let earlier = |best: &(u64, Reading)| from.seq < best.0;
                match cut {
                    true => search.refused = search.refused.max(Some(from.seq)),
                    false if search.best.as_ref().is_none_or(earlier) => {
                        search.best = Some((from.seq, Reading::default()))
                    }
                    false => {}
                }
                match search.next(latest) {
                    Next::Read(seq) => Some(seq),
                    Next::Found | Next::Spent => None,
                }
            }
        };
        if let Some(seq) = from {
            let cursor = Some(Cursor {
                epoch: epoch.to_owned(),
                seq,
            });
            let session = session.clone();
            stream.wanted = Some((Step::Watch { session, cursor }, search));
        }
    }
}
