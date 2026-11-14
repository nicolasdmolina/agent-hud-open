//! How long a request is held: the hold every request gets, and what its
//! running out ends.
use super::{Content, Requests};
use std::sync::PoisonError;

impl Requests {
    pub fn expire(&self, now_ms: i64) {
        let (expired, lapsed) = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut expired = Vec::new();
            let mut index = 0;
            while index < state.pending.len() {
                if state.pending[index].deadline_ms <= now_ms {
                    let item = state.pending.remove(index).expect("pending index exists");
                    if matches!(&item.content, Content::Native(_)) {
                        state.dismissed.insert(item.id.clone());
                    }
                    expired.push(item);
                } else {
                    index += 1;
                }
            }
            // A request whose hold ran out while its answer was on its way
            // is over like any other: nothing gives it back.
            let hold = state.hold_ms as i64;
            state
                .taken
                .retain(|_, taken| taken.created_ms.saturating_add(hold) > now_ms);
            // A reader's reading of its sessions is believed for so long.
            let lapsed = state.native.lapse(now_ms);
            if !expired.is_empty() || lapsed {
                state.revision = state.revision.wrapping_add(1);
            }
            (expired, lapsed)
        };
        if !expired.is_empty() || lapsed {
            for item in expired {
                let _ = item.sender.send(None);
            }
            self.wake();
        }
    }
    pub fn set_hold_ms(&self, hold_ms: u64) {
        let changed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.hold_ms == hold_ms {
                false
            } else {
                state.hold_ms = hold_ms;
                for item in &mut state.pending {
                    item.deadline_ms = item.created_ms.saturating_add(hold_ms as i64);
                }
                true
            }
        };
        if changed {
            self.wake();
        }
    }
}
