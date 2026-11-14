//! Which profiles a gateway serves, as that changes while it is followed.
//! A machine can run several gateways, and a profile can leave one of
//! them: retired by it, or served by another instance. Whoever looks at
//! every gateway of the machine tells this model what it found; the model
//! closes the cards of a profile that left, and opens none for a profile
//! two instances claim at once.
use super::connection::{Connection, Instance};
use super::effect::{Closed, Effect};
use super::identity::Profile;
use super::state::Gateway;
use std::collections::BTreeSet;

impl Instance {
    /// Whether the instance serves `profile`.
    pub fn serves(&self, profile: &Profile) -> bool {
        self.served.iter().any(|served| served.profile == *profile)
    }
}

impl Gateway {
    /// The instance that was last ready here, by its id: the one run of
    /// the gateway this model's cards were asked by.
    pub fn instance(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    /// The profiles the gateway served when it was last ready, less the
    /// ones retired since. Kept while it cannot be reached: what waits in
    /// them still waits.
    pub fn holds(&self) -> &BTreeSet<Profile> {
        &self.held
    }

    /// The profiles no card is opened for just now.
    pub fn withheld(&self) -> &BTreeSet<Profile> {
        &self.withheld
    }

    /// `gone` are no longer this gateway's. Their cards close, their
    /// sessions are forgotten, and no poll lists them again until the
    /// gateway itself says it serves them. A poll under way may still list
    /// one: it is dropped.
    pub(super) fn retire(&mut self, gone: &BTreeSet<Profile>, effects: &mut Vec<Effect>) {
        let left = |profile: &Profile| gone.contains(profile);
        let cards: Vec<_> = self.cards.keys().cloned().collect();
        let held = self.held.iter().any(left) || cards.iter().any(|(at, _)| left(&at.profile));
        if !held {
            return;
        }
        self.polls += 1;
        self.held.retain(|profile| !left(profile));
        self.listed.retain(|at, _| !left(&at.profile));
        self.snapshotted.retain(|at, _| !left(&at.profile));
        for key in cards.iter().filter(|(at, _)| left(&at.profile)) {
            self.close(key, Closed::Retired, effects);
        }
        if let Connection::Ready(instance) = &mut self.connection {
            instance.served.retain(|served| !left(&served.profile));
        }
    }

    /// `contested` are the profiles more than one instance claims. A poll
    /// goes on listing them here, and the cards that show are kept and
    /// closed as Hermes says; no new card opens for one of them. Once a
    /// profile is no longer contested, a poll is asked for at once, which
    /// opens what waits in it.
    pub(super) fn withhold(&mut self, contested: &BTreeSet<Profile>, effects: &mut Vec<Effect>) {
        let freed = self.withheld.iter().any(|was| !contested.contains(was));
        self.withheld = contested.clone();
        if freed {
            effects.push(Effect::Poll);
        }
    }
}
