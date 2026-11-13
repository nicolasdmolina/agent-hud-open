//! What native channels tell of their sessions, each reading kept with the
//! reader that made it. One source can have several readers (a channel for
//! each process it follows), and no two of them read at the same moment:
//! one time for them all would let a late reader make another's old reading
//! look new, or a stalled reader lapse a healthy one. So a reader replaces
//! only what it told itself, and its reading has its own time and lapses by
//! itself.
use super::Requests;
use agent_hud_agents::{LiveState, LiveStates};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::PoisonError,
};

/// How long a native channel's reading of its sessions is believed after it
/// came back. The channel that tells states today reads its client every
/// second while anything runs and every ten seconds when all is idle;
/// thirty seconds is three idle readings, room for a slow one. A channel that stops reading without a word
/// (a call that hangs on a socket that still looks open) gives its sessions
/// back to their hooks and logs by then.
pub const LIVE_STATES_MS: i64 = 30_000;

/// What one reader last said of its live sessions, and when it read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeStates {
    pub read_ms: i64,
    pub sessions: HashMap<String, LiveState>,
}

impl NativeStates {
    /// Whether the reading is still believed at `now_ms`.
    pub fn fresh(&self, now_ms: i64) -> bool {
        now_ms.saturating_sub(self.read_ms) < LIVE_STATES_MS
    }
}

/// Every reader's reading, by source and then by reader.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeReadings(HashMap<String, BTreeMap<String, NativeStates>>);

impl NativeReadings {
    /// What `reader` last read of `source`'s sessions, while it is held.
    pub fn of(&self, source: &str, reader: &str) -> Option<&NativeStates> {
        self.0.get(source)?.get(reader)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// What a session is doing and when that was read: from the reading
    /// that lists it, the newest one where several do, and only one still
    /// believed at `now_ms`. Two readings of the same moment are settled by
    /// the readers' names, so the order they arrived in never matters.
    pub fn told(&self, source: &str, id: &str, now_ms: i64) -> Option<(LiveState, i64)> {
        let readings = self.0.get(source)?.values();
        readings
            .filter(|reading| reading.fresh(now_ms))
            .filter_map(|reading| Some((*reading.sessions.get(id)?, reading.read_ms)))
            .max_by_key(|(_, read_ms)| *read_ms)
    }
    fn entry(&mut self, source: &str, reader: &str) -> Option<&mut NativeStates> {
        self.0.get_mut(source)?.get_mut(reader)
    }
    fn remove(&mut self, source: &str, reader: &str) -> bool {
        let Some(readers) = self.0.get_mut(source) else {
            return false;
        };
        let held = readers.remove(reader).is_some();
        if readers.is_empty() {
            self.0.remove(source);
        }
        held
    }
}

/// What core holds of every reader: the sessions each has a card waiting
/// in, and what each read its sessions doing.
#[derive(Default)]
pub(super) struct Readers {
    waiting: HashMap<String, BTreeMap<String, HashSet<String>>>,
    states: NativeReadings,
}

impl Readers {
    pub(super) fn is_empty(&self) -> bool {
        self.waiting.is_empty() && self.states.is_empty()
    }
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
    /// When each reading lapses.
    pub(super) fn deadlines(&self) -> impl Iterator<Item = i64> + '_ {
        let readings = self.states.0.values().flat_map(BTreeMap::values);
        readings.map(|reading| reading.read_ms.saturating_add(LIVE_STATES_MS))
    }
    /// Drops every reading no longer believed at `now_ms`, each by its own
    /// time; whether any went.
    pub(super) fn lapse(&mut self, now_ms: i64) -> bool {
        let mut lapsed = false;
        self.states.0.retain(|_, readers| {
            let held = readers.len();
            readers.retain(|_, reading| reading.fresh(now_ms));
            lapsed |= readers.len() != held;
            !readers.is_empty()
        });
        lapsed
    }
    fn set_waiting(&mut self, source: &str, reader: &str, next: HashSet<String>) -> bool {
        let old = self.waiting.get(source).and_then(|all| all.get(reader));
        if old.is_some_and(|old| *old == next) || (old.is_none() && next.is_empty()) {
            return false;
        }
        if next.is_empty() {
            self.end_waiting(source, reader);
        } else {
            let readers = self.waiting.entry(source.into()).or_default();
            readers.insert(reader.into(), next);
        }
        true
    }
    fn end_waiting(&mut self, source: &str, reader: &str) -> bool {
        let Some(readers) = self.waiting.get_mut(source) else {
            return false;
        };
        let held = readers.remove(reader).is_some();
        if readers.is_empty() {
            self.waiting.remove(source);
        }
        held
    }
    fn set_states(&mut self, source: &str, reader: &str, next: NativeStates) -> bool {
        match self.states.entry(source, reader) {
            None if next.sessions.is_empty() => false,
            // The same sessions, read again and not lapsed meanwhile.
            Some(old) if old.sessions == next.sessions && old.fresh(next.read_ms) => {
                let newer = next.read_ms > old.read_ms;
                old.read_ms = old.read_ms.max(next.read_ms);
                newer
            }
            _ if next.sessions.is_empty() => self.states.remove(source, reader),
            _ => {
                let readers = self.states.0.entry(source.into()).or_default();
                readers.insert(reader.into(), next);
                true
            }
        }
    }
}

impl Requests {
    /// Applies a change to what the readers told; a real change makes the
    /// views anew and wakes the HUD.
    fn native(&self, change: impl FnOnce(&mut Readers) -> bool) {
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = change(&mut state.native);
            if changed {
                state.revision = state.revision.wrapping_add(1);
            }
            changed
        };
        if changed {
            self.wake();
        }
    }
    /// Native waiting flags may exist without a HUD-answerable request. Keep
    /// phase evidence here without fabricating a card or a transcript timestamp.
    /// The list is `reader`'s whole list for `source`: it replaces what that
    /// reader said before and leaves every other reader's flags.
    pub fn set_native_waiting(&self, source: &str, reader: &str, sessions: &[String], _now: i64) {
        let next: HashSet<String> = sessions
            .iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect();
        self.native(|readers| readers.set_waiting(source, reader, next));
    }
    /// What `reader`, one of `source`'s native channels, says each live
    /// session is doing, whole, as of `states.read_ms`: a session it leaves
    /// out, or an empty list, goes back to what its hooks and its log say
    /// (or to another reader that lists it), and so does every session once
    /// the reading is `LIVE_STATES_MS` old. Another reader's reading is not
    /// touched, and keeps its own time. A channel may call this after
    /// everything it does: the very same reading told again changes
    /// nothing. Any newer reading wakes the HUD, even of the same sessions,
    /// since it decides again against hooks that reported since the last
    /// one (a channel reads at most about once a second).
    pub fn set_native_states(&self, source: &str, reader: &str, states: &LiveStates) {
        let sessions = states.sessions.iter().filter(|(id, _)| !id.is_empty());
        let next = NativeStates {
            read_ms: states.read_ms,
            sessions: sessions.cloned().collect(),
        };
        self.native(|readers| readers.set_states(source, reader, next));
    }
    /// A reader that is over (its channel stopped, or follows something
    /// else now): its states and its waiting flags go at once, and no other
    /// reader's do.
    pub fn end_native_reader(&self, source: &str, reader: &str) {
        self.native(|readers| {
            let waited = readers.end_waiting(source, reader);
            readers.states.remove(source, reader) || waited
        });
    }
    pub fn native_states(&self) -> NativeReadings {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.native.states.clone()
    }
    /// The sessions a native card waits in, by source: every reader's.
    pub fn native_waiting_sessions(&self) -> HashMap<String, HashSet<String>> {
        let state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let sources = state.native.waiting.iter();
        sources
            .map(|(source, readers)| {
                let sessions = readers.values().flatten().cloned().collect();
                (source.clone(), sessions)
            })
            .collect()
    }
}
