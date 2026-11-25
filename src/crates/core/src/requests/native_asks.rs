//! Every request a native channel says waits in a session, the ones it
//! cannot show included, each by the name of its ask (Agent HUD Fork,
//! ticket 74). A request is asked once while its channel keeps it in
//! sight; one whose answer the owner confirmed, that the channel then lost
//! sight of and sees waiting again, is asked anew under another name. The
//! sound decision sounds each ask once that has no card and is not
//! approved, so two requests that wait one after the other in one session
//! each sound, whichever of them shows a card. Nothing here decides what
//! is approved: that stays with the named waits alone (`wait_requests`).
use super::{Readers, Requests};
use std::collections::{HashMap, HashSet};

/// One session's asks: each the id of the request, and the name of its ask.
pub(super) type Asks = HashMap<String, Vec<(String, String)>>;

impl Readers {
    /// Every ask `reader` says waits in each session; whether that changed.
    pub(super) fn set_asks(&mut self, source: &str, reader: &str, next: Asks) -> bool {
        let key = (source.to_owned(), reader.to_owned());
        if self
            .asks
            .get(&key)
            .map_or(next.is_empty(), |old| *old == next)
        {
            return false;
        }
        if next.is_empty() {
            self.asks.remove(&key);
        } else {
            self.asks.insert(key, next);
        }
        true
    }
    /// The requests each waiting session's wait is for (`native_wait_requests`).
    pub(in super::super) fn wait_requests(&self) -> HashMap<String, Vec<String>> {
        let mut all: HashMap<String, Option<Vec<String>>> = HashMap::new();
        for (source, readers) in &self.waiting {
            for (reader, sessions) in readers {
                let named = self.named.get(&(source.clone(), reader.clone()));
                for session in sessions {
                    let told = named.and_then(|named| named.get(session));
                    let entry = all.entry(format!("{source}:{session}"));
                    match (entry.or_insert_with(|| Some(Vec::new())), told) {
                        (Some(requests), Some(told)) => requests.extend(told.iter().cloned()),
                        (requests, _) => *requests = None,
                    }
                }
            }
        }
        all.into_iter()
            .filter_map(|(key, requests)| Some((key, requests?)))
            .collect()
    }
}

impl Requests {
    /// `set_native_waits`, with every request the channel says waits in
    /// each session (`asks`), told in the same change so that no look
    /// falls between the two.
    pub fn set_native_waits_asked(
        &self,
        source: &str,
        reader: &str,
        waits: &[(String, Vec<String>)],
        asks: &[(String, Vec<(String, String)>)],
    ) {
        let waits = waits.iter().filter(|(id, _)| !id.is_empty());
        let next: HashSet<String> = waits.clone().map(|(id, _)| id.clone()).collect();
        let named: HashMap<String, Vec<String>> = waits
            .filter(|(_, requests)| !requests.is_empty())
            .cloned()
            .collect();
        let asks: Asks = asks
            .iter()
            .filter(|(id, asks)| next.contains(id) && !asks.is_empty())
            .cloned()
            .collect();
        self.native(|readers| {
            let named = readers.set_named(source, reader, named);
            let asks = readers.set_asks(source, reader, asks);
            readers.set_waiting(source, reader, next) || named || asks
        });
    }
    /// Every ask that waits in each waiting session, by `source:session`:
    /// what every reader that lists the session says waits in it.
    pub fn native_wait_asks(&self) -> HashMap<String, Vec<(String, String)>> {
        let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let readers = &state.native;
        let mut all: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (source, by_reader) in &readers.waiting {
            for (reader, sessions) in by_reader {
                let told = readers.asks.get(&(source.clone(), reader.clone()));
                for session in sessions {
                    let Some(asks) = told.and_then(|told| told.get(session)) else {
                        continue;
                    };
                    let entry = all.entry(format!("{source}:{session}")).or_default();
                    entry.extend(asks.iter().cloned());
                }
            }
        }
        all
    }
}
