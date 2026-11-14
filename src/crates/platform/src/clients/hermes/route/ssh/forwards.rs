//! The forwarded ports of one far machine: one for each gateway, made
//! once and used by every socket to it, the polls' and each visit's.
//!
//! A forward is the instance's: a gateway that restarts is another
//! instance, on a port that may be another, so its forward is made anew
//! and the one of the instance that answered at that home before is
//! cancelled. A forward lives on one link and is gone with it.
//!
//! Making one asks the master, which can take a moment. Whoever asked may
//! stop meanwhile: the wait ends at once, and the forward, if it still
//! arrives, is cancelled by the thread that made it.
use super::super::Stop;
use crate::ssh_link::{Forward, Link};
use agent_hud_agents::hermes::gateway::{Failure, Instance};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::Duration,
};

/// How long one look at a forward being made lasts.
const TICK: Duration = Duration::from_millis(10);

#[derive(Default)]
pub(super) struct Forwards {
    held: Mutex<Held>,
    changed: Condvar,
}

#[derive(Default)]
struct Held {
    /// The number of the link these are on.
    link: u64,
    by_instance: HashMap<String, Entry>,
    /// Numbers each making, so that one given up is known as that.
    makings: u64,
    /// How many forwards were ever made, for the tests.
    #[cfg(test)]
    made: u64,
}

struct Entry {
    /// The home whose socket answered as this instance (`profile_id`).
    home: String,
    state: Made,
}

enum Made {
    Making(u64),
    Ready(Forward),
}

impl Forwards {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many forwards were ever made.
    #[cfg(test)]
    pub(super) fn made(&self) -> u64 {
        self.held().made
    }

    /// The local port that leads to `instance` on the link `number`.
    pub(super) fn local(&self, number: u64, instance: &str) -> Option<u16> {
        let held = self.held();
        match held.by_instance.get(instance) {
            Some(Entry {
                state: Made::Ready(forward),
                ..
            }) if held.link == number => Some(forward.local),
            _ => None,
        }
    }

    /// Has a forward to `port` on the far loopback in place for
    /// `instance`, on `link`, which is the link `number`.
    pub(super) fn prepare(
        self: &Arc<Self>,
        link: &Arc<Link>,
        number: u64,
        instance: &Instance,
        port: u16,
        stop: &dyn Stop,
    ) -> Result<(), Failure> {
        let id = instance.id.clone();
        let mut held = self.held();
        if held.link != number {
            // Whatever was on another link went with it.
            held.by_instance.clear();
            held.link = number;
        }
        let mut ours = None;
        loop {
            let state = held.by_instance.get(&id).map(|entry| &entry.state);
            match (state, ours) {
                (Some(Made::Ready(_)), _) => return Ok(()),
                // Being made, by this call or another: waited for below.
                (Some(Made::Making(_)), _) => {}
                // The making this call began came to nothing.
                (None, Some(_)) => return Err(Failure::Unreachable),
                (None, None) => {
                    held.makings += 1;
                    let making = held.makings;
                    let home = instance.profile_id.clone();
                    let state = Made::Making(making);
                    held.by_instance.insert(id.clone(), Entry { home, state });
                    ours = Some(making);
                    let (all, link, of) = (self.clone(), link.clone(), id.clone());
                    let made = thread::Builder::new()
                        .name("Hermes far forward".into())
                        .spawn(move || all.make(&link, number, &of, port, making));
                    if made.is_err() {
                        held.by_instance.remove(&id);
                        return Err(Failure::Unreachable);
                    }
                }
            }
            drop(held);
            if stop.wait(Duration::ZERO) {
                // Given up: the forward is cancelled when it arrives.
                if let Some(making) = ours {
                    self.give_up(&id, making);
                }
                return Err(Failure::Unreachable);
            }
            let waiting = self.held();
            let (waited, _) = self
                .changed
                .wait_timeout(waiting, TICK)
                .unwrap_or_else(PoisonError::into_inner);
            held = waited;
            if held.link != number {
                return Err(Failure::Unreachable);
            }
        }
    }

    fn give_up(&self, id: &str, making: u64) {
        let mut held = self.held();
        let own = |entry: &Entry| matches!(entry.state, Made::Making(of) if of == making);
        if held.by_instance.get(id).is_some_and(own) {
            held.by_instance.remove(id);
        }
        drop(held);
        self.changed.notify_all();
    }

    /// Makes the forward, on a thread of its own. The forward of another
    /// instance that answered at the same home goes first.
    fn make(&self, link: &Link, number: u64, id: &str, port: u16, making: u64) {
        let stale: Vec<Forward> = {
            let mut held = self.held();
            let home = held.by_instance.get(id).map(|entry| entry.home.clone());
            let old = |(of, entry): (&String, &Entry)| match &entry.state {
                Made::Ready(forward) if of != id && Some(&entry.home) == home.as_ref() => {
                    Some((of.clone(), *forward))
                }
                _ => None,
            };
            let found: Vec<(String, Forward)> = held.by_instance.iter().filter_map(old).collect();
            for (of, _) in &found {
                held.by_instance.remove(of);
            }
            found.into_iter().map(|(_, forward)| forward).collect()
        };
        for forward in stale {
            let _ = link.cancel(forward);
        }
        let far = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let made = link.forward(far);
        let mut held = self.held();
        let own = |entry: &Entry| matches!(entry.state, Made::Making(of) if of == making);
        let wanted = held.link == number && held.by_instance.get(id).is_some_and(own);
        match (made, wanted) {
            (Ok(forward), true) => {
                #[cfg(test)]
                {
                    held.made += 1;
                }
                if let Some(entry) = held.by_instance.get_mut(id) {
                    entry.state = Made::Ready(forward);
                }
            }
            (Ok(forward), false) => {
                drop(held);
                let _ = link.cancel(forward);
                return self.changed.notify_all();
            }
            (Err(_), true) => {
                held.by_instance.remove(id);
            }
            (Err(_), false) => {}
        }
        drop(held);
        self.changed.notify_all();
    }
}
