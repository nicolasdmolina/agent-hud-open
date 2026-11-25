//! The waits open when the app quits, kept across a relaunch (Agent HUD
//! Fork, ticket 75). A wait is seen to end when a look no longer sees it;
//! one that ends while the app is closed is not, and the next wait under
//! its base would go by the same name, decided within the day, and make
//! no sound. So the base names each session's wait goes by are saved, and
//! at the first look after a relaunch that lists the session, each it no
//! longer waits under is taken as ended. A session no look lists yet (its
//! client not reached) keeps its waits open, as a request decided before
//! a relaunch does not sound after it (ticket 33).
use super::{Tracker, Wait, KEPT_MS};
use std::collections::HashMap;

/// The base names of each session's open wait, and when they were saved.
pub(super) type Open = HashMap<String, (Vec<String>, i64)>;

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
}

impl Tracker {
    /// A wait no look sees any more: each name it went by is over, and
    /// core is told which approved requests' wait ended.
    pub(super) fn close(&mut self, key: String, wait: Wait, now: i64) {
        for base in wait.bases(&key) {
            self.spent(Some(&wait), &base);
            self.end(Some(base), now);
        }
        let approved = wait.approved.into_iter();
        self.ended
            .extend(approved.map(|request| (key.clone(), request)));
    }

    /// At the first look since a relaunch that lists session `key`, which
    /// waits under the bases `current`: each base its wait went by when
    /// the app quit and goes by no more is over.
    pub(super) fn reopen(&mut self, key: &str, current: &[String], now: i64) {
        let Some((bases, _)) = self.restored.remove(key) else {
            return;
        };
        for base in bases.into_iter().filter(|base| !current.contains(base)) {
            self.end(Some(base), now);
        }
    }

    /// The open waits to save: this look's, and those restored whose
    /// session no look has listed since, for a day.
    pub(super) fn open(&mut self, now: i64) -> Open {
        self.restored.retain(|_, (_, at)| now - *at < KEPT_MS);
        let mut open = self.restored.clone();
        for (key, wait) in &self.waits {
            let bases = wait.bases(key);
            if !bases.is_empty() {
                open.insert(key.clone(), (bases, now));
            }
        }
        open
    }
}
