//! The waits open when the app quits (Agent HUD Fork, ticket 75). A wait
//! is seen to end when a look no longer sees it; one that ends while the
//! app is closed is not, and the next wait under its base would go by the
//! same name, decided within the day, and make no sound. So the base names
//! each session's wait goes by are saved, and when the file is loaded each
//! is taken as ended. A wait still there at the first look is only noted,
//! as everything that waits at that look is; one seen only later (its
//! client reached after the launch) is told again: an extra ping at most.
//! A plain name an open wait went by with no card is forgotten at load, as
//! when a look sees such a wait end (`spent`), so a card of its request
//! after the relaunch is another wait and sounds (ticket 81).
use super::{Tracker, Wait};
use std::collections::HashMap;

/// The base names of each session's open wait, when they were saved, and
/// those of them it went by with no card.
pub(super) type Open = HashMap<String, (Vec<String>, i64, Vec<String>)>;

impl Wait {
    /// The base names this wait of session `key` goes by (`bare.rs`).
    pub(super) fn bases(&self, key: &str) -> Vec<String> {
        let named = self
            .named
            .iter()
            .map(|(r, _)| format!("needs-you:{key}:{r}"));
        let asks = self
            .asks
            .iter()
            .map(|a| format!("needs-you:{key}:{}", a.ask));
        self.lent.iter().cloned().chain(named).chain(asks).collect()
    }

    /// The names that are over once this wait ends: its bases, and the
    /// names of every card shown in it.
    pub(super) fn ended(&self, key: &str) -> Vec<String> {
        let mut names = self.bases(key);
        Wait::add(&mut names, &self.cards);
        names
    }
}

impl Tracker {
    /// A wait no look sees any more: each name it went by is over, its
    /// cards' too, and core is told which approved requests' wait ended.
    pub(super) fn close(&mut self, key: String, wait: Wait, now: i64) {
        for base in wait.ended(&key) {
            self.spent(Some(&wait), &base);
            self.end(Some(base), now);
        }
        let approved = wait.approved.into_iter();
        self.ended
            .extend(approved.map(|request| (key.clone(), request)));
    }

    /// The waits open after a relaunch: each saved one is over.
    pub(super) fn reopen(&mut self, open: Open, now: i64) {
        for (bases, _, uncarded) in open.into_values() {
            for name in uncarded {
                self.changed |= self.decided.remove(&name).is_some();
            }
            for base in bases {
                self.end(Some(base), now);
            }
        }
    }

    /// The open waits to save: this look's.
    pub(super) fn open(&self, now: i64) -> Open {
        let mut open = Open::new();
        for (key, wait) in &self.waits {
            // A wait told by a card alone is not saved (ticket 33's file);
            // one saved goes with its cards' names, which end at the load.
            if wait.bases(key).is_empty() {
                continue;
            }
            let bases = wait.ended(key);
            let went = |b: &&String| wait.uncarded.iter().any(|n| super::bare::under(b, n));
            let uncarded = bases.iter().filter(went).cloned().collect();
            open.insert(key.clone(), (bases, now, uncarded));
        }
        open
    }
}
