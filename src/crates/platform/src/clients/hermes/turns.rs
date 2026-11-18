//! The turns of a far machine's sessions, told to core. This machine's
//! Hermes runs Agent HUD's hooks, which say when a turn starts and ends.
//! A far one runs none, so its gateway's polls read those facts
//! (`agents::hermes::gateway`, `turns.rs`), and each is told here as the
//! very fact a hook of this machine gives, under the far session's id:
//! core cannot tell the two apart, and decides its alerts as for any
//! session.
//!
//! The time a fact is told with is when the poll that read it came back:
//! when the turn's start or end was seen. Nothing Hermes sends says when
//! a turn ended, and this is never offered as that.
//!
//! The looks those polls take at idle sessions are small. One that is
//! slow all the same, or goes unanswered, would hold every poll up, and
//! with the poll every card. So a session whose look was slow is not
//! looked at for a while (`Rests`); its place in its stream is kept, and
//! what its turns did meanwhile is told when it is looked at again.
use super::{Model, Shared};
use agent_hud_agents::hermes::gateway::{SessionRef, Step};
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// How long a session is not looked at once a look at it was slow, in
/// the times one call may take (`Pace::answer`): thirty seconds in the app.
const REST: u32 = 6;

/// The sessions that are not looked at just now, each with until when.
#[derive(Default)]
pub(super) struct Rests(Mutex<HashMap<SessionRef, Instant>>);

impl Rests {
    /// Whether `step` is a look at a session that rests. A read that
    /// follows a look in the same poll is never held back: it is what
    /// tells the turn.
    pub(super) fn holds(&self, step: &Step) -> bool {
        let Step::Watch {
            session,
            cursor: None,
        } = step
        else {
            return false;
        };
        let mut rests = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        rests.retain(|_, until| *until > now);
        rests.contains_key(session)
    }

    /// `step` took long, or was never answered: where it is a call for
    /// the turns, its session rests.
    pub(super) fn rest(&self, step: &Step, answer: Duration) {
        if let Step::Watch { session, .. } = step {
            let mut rests = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            rests.insert(session.clone(), Instant::now() + answer * REST);
        }
    }
}

impl Shared {
    /// Tells core each turn the model read to start or end, in order,
    /// with the model locked as for every effect. `seen_ms` is when the
    /// poll that read them came back. A gateway of this machine tells
    /// none: its model reads none, and nobody is there to be told.
    pub(super) fn turned(&self, model: &mut Model, seen_ms: i64) {
        let turns = model.gateway.take_turns();
        let Some(told) = self.turns.as_ref().filter(|_| !turns.is_empty()) else {
            return;
        };
        let label = self.label.lock().unwrap_or_else(PoisonError::into_inner);
        let label = label.clone();
        for turn in turns {
            let word = if turn.ended { "ended" } else { "started" };
            let (session, id) = (turn.session.hud_id(), turn.id());
            self.log
                .line(&format!("turn {word} session={session} turn={id}"));
            told(turn.hook_event(seen_ms, label.as_deref()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_agents::hermes::gateway::{Cursor, GatewayId, Machine, Profile};

    /// A session whose look was slow is not looked at for six times as
    /// long as a call may take, and then is again. Only looks are held
    /// back: the read after a look is what tells a turn, also where it
    /// is one of several that search for the oldest event still kept.
    #[test]
    fn a_rest_holds_back_looks_and_never_the_read_after_one() {
        let gateway = GatewayId::new(Machine::This, Profile::Default);
        let at = |id: &str| SessionRef::new(gateway.clone(), Profile::Default, id).unwrap();
        let look = |id: &str| Step::Watch {
            session: at(id),
            cursor: None,
        };
        let cursor = Some(Cursor {
            epoch: "epoch-1".into(),
            seq: 7,
        });
        let read = Step::Watch {
            session: at("slow"),
            cursor,
        };
        let rests = Rests::default();
        assert!(!rests.holds(&look("slow")));
        // A card's own read is no call for the turns: it rests nobody.
        let since = Step::Since {
            session: at("slow"),
            cursor: None,
        };
        rests.rest(&since, Duration::from_secs(5));
        assert!(!rests.holds(&look("slow")));

        rests.rest(&read, Duration::from_millis(20));
        assert!(rests.holds(&look("slow")));
        assert!(!rests.holds(&read));
        assert!(!rests.holds(&look("other")));
        assert!(!rests.holds(&since));
        std::thread::sleep(Duration::from_millis(20 * REST as u64 + 30));
        assert!(!rests.holds(&look("slow")));
    }
}
