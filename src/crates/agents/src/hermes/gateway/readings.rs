//! What Agent HUD reads from the gateway on request: its list of live
//! sessions, a replay of one session's stream, and a snapshot of one.
//!
//! The three say what waits, and they are not equally complete. A replay
//! carries the requests asked since its cursor and every set of questions
//! open now, but not a question or an approval whose request event the
//! stream no longer holds (it keeps 512 events and 4 MiB a session, for 64
//! sessions and 64 MiB in all). Only a snapshot lists those, so a replay
//! that is cut short asks for one.
use super::ask::Ask;
use super::effect::{Closed, Effect, Status};
use super::frame::{event, Cursor};
use super::identity::{Profile, SessionRef};
use super::state::Gateway;
use serde_json::Value;
use std::collections::BTreeSet;

/// How long a sent answer to a set of questions may go unconfirmed before a
/// reading that still lists the set means the answer was dropped.
const UNCONFIRMED_MS: i64 = 5_000;

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
            if busy || !self.asked(&at, |_| true).is_empty() {
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
        let prompts = !self.asked(&at, |ask| ask.generation().is_some()).is_empty();
        let session = self.entry_mut(profile, id);
        let known = session.cursor.as_ref().map(|cursor| cursor.epoch.as_str());
        let resume = Effect::Resume {
            session: at.clone(),
        };
        if result["snapshot_required"] == true {
            match epoch {
                Some(epoch) if known != Some(epoch) => {
                    let cursor = Cursor {
                        epoch: epoch.to_owned(),
                        seq: 0,
                    };
                    session.cursor = Some(cursor.clone());
                    // A card from the stream before cannot be settled by
                    // this one: only a snapshot says whether it still waits.
                    effects.push(if prompts {
                        resume
                    } else {
                        Effect::Read {
                            session: at.clone(),
                            cursor: Some(cursor),
                        }
                    });
                }
                // Cut short: the stream no longer holds the range asked
                // for. A question or an approval can wait unseen only where
                // the session says it waits or a card is showing.
                _ if prompts || session.phase == Status::Waiting => effects.push(resume),
                // Otherwise the reading goes on from the latest event, and
                // no subscription is taken for a session that only works.
                _ => {
                    if let (Some(cursor), Some(latest)) = (&mut session.cursor, latest) {
                        cursor.seq = latest;
                    }
                }
            }
        } else {
            for params in result["events"].as_array().into_iter().flatten() {
                if let Some(event) = event(params) {
                    self.event(profile, &event, now_ms, effects);
                }
            }
            let shows = !self.asked(&at, |_| true).is_empty();
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
                effects.push(resume);
            }
        }
        self.batches(&at, &result["open_requests"], now_ms, effects);
    }

    /// `session.resume`: everything waiting in the session now. What it
    /// lists is shown, and a card it does not list is over.
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
        let session = self.entry_mut(profile, id);
        session.subscription = crate::text(&result["subscription_id"]).map(str::to_owned);
        if let Some(epoch) = crate::text(&result["replay_epoch"]) {
            session.cursor = Some(Cursor {
                epoch: epoch.to_owned(),
                seq: result["last_sequence"].as_u64().unwrap_or(0),
            });
        }
        session.snapshot_at = session.cursor.clone();
        if let Some(entries) = result["prompts"].as_array() {
            let prompts: Vec<(Ask, Option<i64>)> = entries.iter().filter_map(Ask::prompt).collect();
            let listed: BTreeSet<String> = prompts
                .iter()
                .map(|(ask, _)| at.request_id(ask.key()))
                .collect();
            for id in self.asked(&at, |ask| ask.generation().is_some()) {
                if !listed.contains(&id) {
                    self.close(&id, Closed::Gone, effects);
                }
            }
            for (ask, wait_ms) in prompts {
                let deadline = wait_ms.map(|wait| now_ms.saturating_add(wait));
                self.show(at.clone(), ask, deadline, effects);
            }
        }
        self.batches(&at, &result["open_requests"], now_ms, effects);
    }

    /// `open_requests` is every set of questions open in the session now.
    /// One no longer listed is over. One still listed long after its answer
    /// was sent never got the answer, which Hermes drops without a word.
    fn batches(&mut self, at: &SessionRef, open: &Value, now_ms: i64, effects: &mut Vec<Effect>) {
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
            if !listed.contains(&id) {
                self.close(&id, Closed::Gone, effects);
            }
        }
        for ask in asks {
            let id = at.request_id(ask.key());
            match self.open.get_mut(&id) {
                Some(open)
                    if open
                        .sent_ms
                        .is_some_and(|sent| now_ms - sent >= UNCONFIRMED_MS) =>
                {
                    open.sent_ms = None;
                    effects.push(Effect::Unanswered {
                        pending: open.pending.clone(),
                        why: None,
                    });
                }
                Some(_) => {}
                None => self.show(at.clone(), ask, None, effects),
            }
        }
    }
}
