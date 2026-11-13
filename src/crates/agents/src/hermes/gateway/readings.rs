//! What Agent HUD reads from the gateway on request: its list of live
//! sessions, a replay of one session's stream, and a snapshot of one.
//!
//! The three say what waits, and they are not equally complete. A replay
//! carries the requests asked since its cursor and every set of questions
//! open now, but not a question or an approval whose request event the
//! stream no longer holds (it keeps 512 events and 4 MiB a session, for 64
//! sessions and 64 MiB in all). Only a snapshot lists those, so a replay
//! that is cut short asks for one.
//!
//! A reading is true of one place in the stream, and events newer than it
//! may already have been seen. So it never moves the cursor back, it
//! closes a card only if the card was last seen at or before its place,
//! and it never shows a request already known to be over.
use super::ask::Ask;
use super::cards::Heard;
use super::effect::{Closed, Effect, Status};
use super::frame::{event, Cursor};
use super::identity::{Profile, SessionRef};
use super::state::Gateway;
use serde_json::Value;
use std::collections::BTreeSet;

/// How long a sent answer to a set of questions may go unconfirmed before a
/// reading that still lists the set means the answer was dropped.
const UNCONFIRMED_MS: i64 = 5_000;

/// Whether a reading at `place` covers something last seen at `seen`: it
/// was there to be listed. Another stream's requests are always covered.
fn covers(place: &Option<Cursor>, seen: &Option<Cursor>, at_the_same_place: bool) -> bool {
    match (place, seen) {
        (Some(place), Some(seen)) if place.epoch == seen.epoch => {
            seen.seq < place.seq || (at_the_same_place && seen.seq == place.seq)
        }
        _ => true,
    }
}

impl Gateway {
    /// `session.active_list`: which sessions live in this profile and what
    /// each is doing. Every session that works, waits or still shows a card
    /// is read; one that left the list is over, and so are its requests.
    pub(super) fn listed(&mut self, profile: &Profile, result: &Value, effects: &mut Vec<Effect>) {
        let Some(rows) = result["sessions"].as_array() else {
            return;
        };
        let mut live = BTreeSet::new();
        for row in rows {
            let Some(id) = crate::text(&row["id"]).or_else(|| crate::text(&row["session_id"]))
            else {
                continue;
            };
            let phase = row["status"].as_str().and_then(Status::parse);
            let session = self.entry_mut(profile, id);
            session.phase = phase.unwrap_or(session.phase);
            let cursor = session.cursor.clone();
            let at = self.session(profile, id);
            let busy = matches!(phase, Some(Status::Working | Status::Waiting));
            if self.strays(&at) {
                Self::resume(&at, effects);
            } else if busy || !self.asked(&at, |_| true).is_empty() {
                effects.push(Effect::Read {
                    session: at,
                    cursor,
                });
            }
            live.insert(id.to_owned());
        }
        let ended: Vec<String> = self
            .sessions
            .keys()
            .filter(|(of, id)| of == profile && !live.contains(id))
            .map(|(_, id)| id.clone())
            .collect();
        for id in ended {
            for request in self.asked(&self.session(profile, &id), |_| true) {
                self.close(&request, Closed::SessionEnded, effects);
            }
            self.sessions.remove(&(profile.clone(), id));
        }
    }

    /// `session.events.since`. The first reading of a stream only learns
    /// its epoch, and the next starts at 0 in it, never at its latest
    /// event: starting there would skip a request that already waits.
    pub(super) fn replayed(
        &mut self,
        profile: &Profile,
        id: &str,
        result: &Value,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let at = self.session(profile, id);
        let epoch = crate::text(&result["replay_epoch"]).or_else(|| crate::text(&result["epoch"]));
        let latest = result["last_sequence"]
            .as_u64()
            .or(result["latest_seq"].as_u64());
        let place = epoch.zip(latest).map(|(epoch, seq)| Cursor {
            epoch: epoch.to_owned(),
            seq,
        });
        let prompts = !self.asked(&at, |ask| ask.generation().is_some()).is_empty();
        let session = self.entry_mut(profile, id);
        if epoch.is_some_and(|epoch| session.left.iter().any(|left| left == epoch)) {
            return;
        }
        let known = session.cursor.as_ref().map(|cursor| cursor.epoch.as_str());
        if result["snapshot_required"] == true {
            match epoch {
                Some(epoch) if known != Some(epoch) => {
                    let cursor = Cursor {
                        epoch: epoch.to_owned(),
                        seq: 0,
                    };
                    if let Some(before) = session.cursor.replace(cursor.clone()) {
                        session.left.push(before.epoch);
                    }
                    // A card from the stream before cannot be settled by
                    // this one: only a snapshot says whether it still waits.
                    if prompts {
                        Self::resume(&at, effects);
                    } else {
                        effects.push(Effect::Read {
                            session: at.clone(),
                            cursor: Some(cursor),
                        });
                    }
                }
                // Cut short: the stream no longer holds the range asked
                // for. A question or an approval can wait unseen only where
                // the session says it waits or a card is showing.
                _ if prompts || session.phase == Status::Waiting => Self::resume(&at, effects),
                // Otherwise the reading goes on from the latest event, and
                // no subscription is taken for a session that only works.
                _ => {
                    if let (Some(cursor), Some(latest)) = (&mut session.cursor, latest) {
                        cursor.seq = cursor.seq.max(latest);
                        session.skipped = Some(cursor.clone());
                    }
                }
            }
        } else {
            for params in result["events"].as_array().into_iter().flatten() {
                if let Some(event) = event(params) {
                    self.event(profile, &event, Heard::Later, now_ms, effects);
                }
            }
            let shows = !self.asked(&at, |_| true).is_empty();
            let strays = self.strays(&at);
            let session = self.entry_mut(profile, id);
            if let (Some(cursor), Some(latest)) = (&mut session.cursor, latest) {
                cursor.seq = cursor.seq.max(latest);
            }
            // It says it waits, and a whole reading shows nothing it waits
            // on: a request the stream never held. Asked once per position.
            let unexplained = session.phase == Status::Waiting
                && !shows
                && session.retired.is_empty()
                && session.snapshot_at != session.cursor;
            if unexplained {
                session.snapshot_at = session.cursor.clone();
            }
            if unexplained || strays {
                Self::resume(&at, effects);
            }
        }
        self.batches(&at, &result["open_requests"], place, now_ms, effects);
    }

    /// `session.resume`: everything that waited in the session at one place
    /// in its stream. A card last seen by then that it does not list is
    /// over; what it lists is shown, unless known to be over since.
    pub(super) fn resumed(
        &mut self,
        profile: &Profile,
        result: &Value,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(id) = crate::text(&result["session_id"]) else {
            return;
        };
        let at = self.session(profile, id);
        let place = crate::text(&result["replay_epoch"]).map(|epoch| Cursor {
            epoch: epoch.to_owned(),
            seq: result["last_sequence"].as_u64().unwrap_or(0),
        });
        let session = self.entry_mut(profile, id);
        if let Some(place) = &place {
            // Taken in a stream the session has left, or before events that
            // were never read: it arrives late and cannot be merged.
            let unread = |skipped: &Cursor| skipped.epoch == place.epoch && place.seq < skipped.seq;
            if session.left.contains(&place.epoch) || session.skipped.as_ref().is_some_and(unread) {
                return;
            }
            match &mut session.cursor {
                Some(cursor) if cursor.epoch == place.epoch => {
                    cursor.seq = cursor.seq.max(place.seq)
                }
                cursor => {
                    if let Some(before) = cursor.replace(place.clone()) {
                        session.left.push(before.epoch);
                    }
                }
            }
        }
        session.subscription = crate::text(&result["subscription_id"]).map(str::to_owned);
        session.snapshot_at = session.cursor.clone();
        session.active = session.active.max(result["execution_generation"].as_u64());
        if let Some(entries) = result["prompts"].as_array() {
            let prompts: Vec<(Ask, Option<i64>)> = entries.iter().filter_map(Ask::prompt).collect();
            let listed: BTreeSet<String> = prompts
                .iter()
                .map(|(ask, _)| at.request_id(ask.key()))
                .collect();
            for id in self.asked(&at, |ask| ask.generation().is_some()) {
                let seen = &self.open[&id].seen;
                if !listed.contains(&id) && covers(&place, seen, true) {
                    self.close(&id, Closed::Gone, effects);
                }
            }
            for (ask, wait_ms) in prompts {
                let seen = place.clone();
                self.show(
                    at.clone(),
                    ask,
                    seen,
                    Heard::Later,
                    wait_ms,
                    now_ms,
                    effects,
                );
            }
        }
        self.batches(&at, &result["open_requests"], place, now_ms, effects);
    }

    /// `open_requests` is every set of questions open in the session at the
    /// reading's place. One last seen before that place and not listed is
    /// over. One still listed long after its answer was sent never got the
    /// answer, which Hermes drops without a word.
    fn batches(
        &mut self,
        at: &SessionRef,
        open: &Value,
        place: Option<Cursor>,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(entries) = open.as_array() else {
            return;
        };
        let asks: Vec<Ask> = entries
            .iter()
            .filter_map(|entry| {
                let (id, method) = (entry["id"].as_str()?, entry["method"].as_str()?);
                Ask::batch(id, method, &entry["params"])
            })
            .collect();
        let listed: BTreeSet<String> = asks.iter().map(|ask| at.request_id(ask.key())).collect();
        for id in self.asked(at, |ask| ask.generation().is_none()) {
            // A request has no place of its own in the stream, so a
            // reading at the very place it was seen may predate it.
            if !listed.contains(&id) && covers(&place, &self.open[&id].seen, false) {
                self.close(&id, Closed::Gone, effects);
            }
        }
        for ask in asks {
            let id = at.request_id(ask.key());
            let dropped = self.open.get_mut(&id).is_some_and(|open| {
                let late = open
                    .sent_ms
                    .is_some_and(|sent| now_ms - sent >= UNCONFIRMED_MS);
                if late {
                    open.sent_ms = None;
                }
                late
            });
            let seen = place.clone();
            self.show(at.clone(), ask, seen, Heard::Later, None, now_ms, effects);
            if let (true, Some(open)) = (dropped, self.open.get(&id)) {
                effects.push(Effect::Unanswered {
                    pending: open.pending.clone(),
                    why: None,
                });
            }
        }
    }
}
