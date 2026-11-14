//! One ephemeral owner for requests waiting on the user. Client adapters own
//! decoding/encoding; transports own connections. No request body is persisted.
use agent_hud_agents::{Machines, Request};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{mpsc, Arc, Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

mod answer;
mod back;
mod deliveries;
mod hold;
mod native;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unreachable_tests;
mod views;

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Requests(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    wake: Mutex<Option<Wake>>,
}
struct State {
    epoch: u128,
    sequence: u64,
    /// Counts the waits begun: each registration and each put-back is one.
    waits: u64,
    revision: u64,
    hold_ms: u64,
    pending: VecDeque<Pending>,
    dismissed: HashSet<String>,
    /// The native requests a decision was taken for, each until its
    /// adapter sent it or gave the request back (`back.rs`).
    taken: HashMap<String, back::Taken>,
    reviewers: HashMap<(String, String, String), Reviewer>,
    deliveries: VecDeque<Value>,
    /// What each native channel's readers say of their sessions.
    native: native::Readers,
    /// What each source's native channel last said of its machines.
    machines: BTreeMap<String, Machines>,
}
pub use native::{NativeReadings, NativeStates, LIVE_STATES_MS};
struct Reviewer {
    value: Option<String>,
    at: i64,
}
// Only a handful of requests wait at a time, so the larger variant stays inline.
#[allow(clippy::large_enum_variant)]
enum Content {
    Hook(Request),
    Native(Value),
}
struct Pending {
    id: String,
    content: Content,
    created_ms: i64,
    deadline_ms: i64,
    sender: mpsc::Sender<Option<Value>>,
    wait: u64,
}
pub struct Ticket {
    pub id: String,
    pub response: mpsc::Receiver<Option<Value>>,
    pub deadline_ms: i64,
    /// Which wait of the request this is. A decision arrives through the
    /// ticket of the wait it was taken in, and only that wait can be given
    /// back (`Requests::put_back`).
    pub wait: u64,
}

impl Default for Requests {
    fn default() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                epoch: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                sequence: 0,
                waits: 0,
                revision: 0,
                hold_ms: 600_000,
                pending: VecDeque::new(),
                dismissed: HashSet::new(),
                taken: HashMap::new(),
                reviewers: HashMap::new(),
                deliveries: VecDeque::new(),
                native: native::Readers::default(),
                machines: BTreeMap::new(),
            }),
            wake: Mutex::new(None),
        }))
    }
}

impl Requests {
    pub fn set_waker(&self, wake: Wake) {
        *self.0.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }
    fn wake(&self) {
        let wake = self
            .0
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }
    pub fn register(&self, request: Request, now_ms: i64) -> Ticket {
        let (sender, response) = mpsc::channel();
        let ticket = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.sequence = state.sequence.wrapping_add(1);
            let id = format!("hook:{}:{}:{}", request.source, state.epoch, state.sequence);
            let deadline_ms = now_ms.saturating_add(state.hold_ms as i64);
            let wait = state.begin_wait();
            state.pending.push_back(Pending {
                id: id.clone(),
                content: Content::Hook(request),
                created_ms: now_ms,
                deadline_ms,
                sender,
                wait,
            });
            state.revision = state.revision.wrapping_add(1);
            Ticket {
                id,
                response,
                deadline_ms,
                wait,
            }
        };
        self.wake();
        ticket
    }
    /// Native adapters provide the standardized display and receive the user's
    /// standardized decision. Their source-specific reply remains in the adapter.
    pub fn register_native(&self, id: String, mut view: Value, now_ms: i64) -> Option<Ticket> {
        let (sender, response) = mpsc::channel();
        let ticket = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.dismissed.contains(&id) || state.pending.iter().any(|p| p.id == id) {
                return None;
            }
            let deadline_ms = now_ms.saturating_add(state.hold_ms as i64);
            view["id"] = Value::String(id.clone());
            let wait = state.begin_wait();
            state.pending.push_back(Pending {
                id: id.clone(),
                content: Content::Native(view),
                created_ms: now_ms,
                deadline_ms,
                sender,
                wait,
            });
            state.revision = state.revision.wrapping_add(1);
            Ticket {
                id,
                response,
                deadline_ms,
                wait,
            }
        };
        self.wake();
        Some(ticket)
    }
    /// Gives a waiting native request its adapter's newer card, such as a
    /// reason it can only be answered in the client that changed; its wait
    /// and its ticket stay.
    pub fn refresh_native(&self, id: &str, mut view: Value) {
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            view["id"] = Value::String(id.into());
            match state.pending.iter_mut().find(|p| p.id == id) {
                Some(Pending {
                    content: Content::Native(shown),
                    ..
                }) if *shown != view => {
                    *shown = view;
                    state.revision = state.revision.wrapping_add(1);
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.wake();
        }
    }
    pub fn cancel(&self, id: &str) {
        let removed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            // Withdrawn: a decision taken for it is given back to nobody.
            state.taken.remove(id);
            let item = state
                .pending
                .iter()
                .position(|p| p.id == id)
                .and_then(|index| state.pending.remove(index));
            if item.is_some() {
                state.revision = state.revision.wrapping_add(1);
            }
            item
        };
        if let Some(item) = removed {
            let _ = item.sender.send(None);
            self.wake();
        }
    }
    /// Explicit source settlement removes a dismissal as well as its live card.
    pub fn settle_native(&self, id: &str) {
        self.cancel(id);
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dismissed
            .remove(id);
    }
    pub fn clear(&self) {
        let (pending, changed) = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let changed =
                !state.pending.is_empty() || !state.native.is_empty() || !state.machines.is_empty();
            if changed {
                state.revision = state.revision.wrapping_add(1);
            }
            state.native.clear();
            state.machines.clear();
            state.dismissed.clear();
            state.taken.clear();
            (std::mem::take(&mut state.pending), changed)
        };
        for item in pending {
            let _ = item.sender.send(None);
        }
        if changed {
            self.wake();
        }
    }
}
