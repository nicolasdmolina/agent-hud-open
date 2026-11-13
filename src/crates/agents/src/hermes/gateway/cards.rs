//! The requests the model holds: showing one, closing one, remembering one
//! that is over, and what the timer does.
use super::ask::{Ask, Pending};
use super::effect::{Closed, Effect, Status};
use super::frame::Cursor;
use super::identity::{Profile, SessionRef};
use super::state::{Gateway, Open, Session};
use std::collections::{BTreeMap, BTreeSet};

/// How a request reached the model: as it was asked, on a subscribed
/// socket, or read some time after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heard {
    Live,
    Later,
}

/// Whether `new` is a place in the same stream before `old`.
fn before(new: &Option<Cursor>, old: &Option<Cursor>) -> bool {
    matches!((new, old), (Some(new), Some(old)) if new.epoch == old.epoch && new.seq < old.seq)
}

impl Gateway {
    /// Shows a request seen waiting at `seen`. One already shown keeps its
    /// card, and its deadline; a set of questions whose answers from other
    /// viewers changed is shown as it is now. One known to be over, or
    /// asked in a turn that ended, is not shown.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show(
        &mut self,
        at: SessionRef,
        ask: Ask,
        seen: Option<Cursor>,
        heard: Heard,
        wait_ms: Option<i64>,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let id = at.request_id(ask.key());
        if let Some(open) = self.open.get_mut(&id) {
            if before(&seen, &open.seen) {
                return;
            }
            if open.pending.ask != ask && matches!(ask, Ask::Batch(_)) {
                open.pending.ask = ask;
                effects.push(Effect::Changed(open.pending.clone()));
            }
            open.seen = seen.or(open.seen.take());
            return;
        }
        let session = self.entry_mut(&at.profile, &at.id);
        let over = session.retired.contains_key(&id)
            || matches!((ask.generation(), session.ended), (Some(turn), Some(ended)) if turn <= ended);
        if over {
            return;
        }
        let until = wait_ms.map(|wait| now_ms.saturating_add(wait));
        let pending = Pending {
            session: at,
            ask,
            deadline_ms: until.filter(|_| heard == Heard::Live),
            recheck_ms: until.filter(|_| heard == Heard::Later),
        };
        effects.push(Effect::Open(pending.clone()));
        self.open.insert(
            id,
            Open {
                pending,
                sent_ms: None,
                seen,
                wait_ms,
            },
        );
    }

    /// The request is over: its card closes if it has one, and it is
    /// remembered either way.
    pub(super) fn end(
        &mut self,
        at: &SessionRef,
        id: &str,
        why: Closed,
        turn: Option<u64>,
        effects: &mut Vec<Effect>,
    ) {
        if self.open.contains_key(id) {
            self.close(id, why, effects);
        } else {
            self.entry_mut(&at.profile, &at.id)
                .retired
                .insert(id.to_owned(), turn);
        }
    }

    pub(super) fn close(&mut self, id: &str, why: Closed, effects: &mut Vec<Effect>) {
        let Some(open) = self.open.remove(id) else {
            return;
        };
        let at = open.pending.session;
        if let Some(session) = self.sessions.get_mut(&(at.profile.clone(), at.id.clone())) {
            session
                .retired
                .insert(id.to_owned(), open.pending.ask.generation());
        }
        effects.push(Effect::Close {
            id: id.to_owned(),
            session: at.hud_id(),
            why,
        });
    }

    /// Forgets everything about the gateway's sessions, closing every card.
    /// It is another run from here on.
    pub(super) fn reset(&mut self, why: Closed, effects: &mut Vec<Effect>) {
        for id in self.open.keys().cloned().collect::<Vec<_>>() {
            self.close(&id, why.clone(), effects);
        }
        self.sessions.clear();
        self.run.0 += 1;
    }

    /// A deadline that is known closes its card. A recheck closes nothing:
    /// it asks Hermes, by a snapshot, whether the request still waits, and
    /// sets itself again in case it does.
    pub(super) fn tick(&mut self, now_ms: i64, effects: &mut Vec<Effect>) {
        let due = |open: &&Open| open.pending.deadline_ms.is_some_and(|at| at <= now_ms);
        let late: Vec<String> = self
            .open
            .values()
            .filter(due)
            .map(|open| open.pending.id())
            .collect();
        for id in late {
            self.close(&id, Closed::TimedOut, effects);
        }
        let mut unsure = BTreeSet::new();
        for open in self.open.values_mut() {
            if open.pending.recheck_ms.is_some_and(|at| at <= now_ms) {
                open.pending.recheck_ms = open.wait_ms.map(|wait| now_ms.saturating_add(wait));
                unsure.insert(open.pending.session.clone());
            }
        }
        effects.extend(unsure.into_iter().map(|session| Effect::Resume { session }));
    }

    /// The ids of the session's open requests that `which` picks.
    pub(super) fn asked(&self, at: &SessionRef, which: impl Fn(&Ask) -> bool) -> Vec<String> {
        self.open
            .iter()
            .filter(|(_, open)| open.pending.session == *at && which(&open.pending.ask))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Whether a question or an approval of the session was last seen in a
    /// stream that is not its stream now. The stream now cannot say what
    /// became of it; only a snapshot can.
    pub(super) fn strays(&self, at: &SessionRef) -> bool {
        let now = self
            .sessions
            .get(&(at.profile.clone(), at.id.clone()))
            .and_then(|session| session.cursor.as_ref());
        self.open.values().any(|open| {
            open.pending.session == *at
                && open.pending.ask.generation().is_some()
                && matches!((&open.seen, now), (Some(seen), Some(now)) if seen.epoch != now.epoch)
        })
    }

    /// Asks for a snapshot of the session, once in these effects.
    pub(super) fn resume(at: &SessionRef, effects: &mut Vec<Effect>) {
        let wanted = Effect::Resume {
            session: at.clone(),
        };
        if !effects.contains(&wanted) {
            effects.push(wanted);
        }
    }

    pub(super) fn entry_mut(&mut self, profile: &Profile, id: &str) -> &mut Session {
        self.sessions
            .entry((profile.clone(), id.to_owned()))
            .or_insert_with(|| Session {
                phase: Status::Idle,
                cursor: None,
                left: Vec::new(),
                active: None,
                ended: None,
                subscription: None,
                snapshot_at: None,
                skipped: None,
                retired: BTreeMap::new(),
            })
    }
}
