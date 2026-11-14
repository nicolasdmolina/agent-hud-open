//! Finding the gateways of one machine. A machine can run one gateway
//! for all of its profiles or one for each, so every Hermes home the
//! route tells of is asked who is there, and the answers are put together
//! by the instance that gave them: each instance is one gateway, and it
//! serves the profiles its own answer lists.
//!
//! A look takes only what was verified. A home counts when its control
//! socket answered as a ready gateway that serves that very home, the
//! homes compared as the gateway writes them. A home that did not answer,
//! or answered anything else, says nothing: not that its gateway is gone,
//! and not that its profile is free. What a look changes is decided in
//! `settle.rs`, and a look that reached nothing changes nothing.
//!
//! A look that missed a reply is not whole (`Seen::missed`): a control
//! socket was there and did not answer in time, or could not be read or
//! trusted, or its gateway is still starting. Whoever did not answer may
//! still claim what it claimed, so such a look settles no contest.
//!
//! A look is taken when the observer starts and then by the clock, and
//! at once when a follower asks: it lost its socket, it found another
//! instance at its home, its gateway serves other profiles than before,
//! or a call was refused as another gateway's profile. It asks `identify`
//! and nothing else, with nothing locked.
use super::{
    control::{self, Silent},
    followers::Followers,
};
use agent_hud_agents::hermes::gateway::{identify, identify_line, Identified, Instance, Profile};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// What one look found.
#[derive(Default)]
pub(super) struct Seen {
    /// The root home of the machine, as a gateway writes it.
    pub(super) root: String,
    /// Every instance that answered, by its id.
    pub(super) instances: BTreeMap<String, Found>,
    /// Which instance answered at the home of each profile.
    pub(super) at: BTreeMap<Profile, String>,
    /// A home that has a gateway to ask gave no answer to go by: this
    /// look does not know every claim there is.
    pub(super) missed: bool,
}

/// One gateway, as it answered.
pub(super) struct Found {
    pub(super) instance: Instance,
    /// Each home that answered as this instance, by its profile: what it
    /// answered there, and the home as the gateway writes it.
    pub(super) via: BTreeMap<Profile, (Value, String)>,
}

impl Seen {
    /// The instances seen that say they serve `profile`.
    fn claim<'a>(&'a self, profile: &Profile) -> Vec<&'a str> {
        let serves = |found: &&Found| found.instance.serves(profile);
        let id = |found: &'a Found| found.instance.id.as_str();
        self.instances.values().filter(serves).map(id).collect()
    }

    /// The instance that serves `profile`, where exactly one seen does.
    pub(super) fn sole(&self, profile: &Profile) -> Option<&str> {
        match self.claim(profile)[..] {
            [one] => Some(one),
            _ => None,
        }
    }

    /// The profiles more than one instance says it serves: a restart that
    /// overlaps, or two gateways set up for one profile.
    pub(super) fn contested(&self) -> BTreeSet<Profile> {
        let served = |found: &Found| found.instance.served.clone();
        let all = self.instances.values().flat_map(served);
        all.map(|served| served.profile)
            .filter(|profile| self.claim(profile).len() > 1)
            .collect()
    }
}

/// Looks at the machine for as long as the observer runs.
pub(super) fn work(all: Arc<Followers>) {
    let by_clock = all.pace.discover.is_some();
    while all.active() {
        // By the clock, the first look comes at once and each wait below
        // ends in one. Without it, only a look that was asked for.
        if all.look.take() || by_clock {
            if let Some(seen) = look(&all) {
                all.settle(&seen);
            }
            all.look.set_looked();
            // However many ask, looks do not follow each other at once.
            all.look.rest(all.pace.gap, || !all.active());
        }
        all.look.wait(all.pace.discover, || !all.active());
    }
}

/// One look: every home asked once. None where the homes could not be
/// told, which is no look at all.
fn look(all: &Followers) -> Option<Seen> {
    let homes = all.route.homes().ok()?;
    let root = homes
        .iter()
        .find(|home| *home.profile() == Profile::Default)?;
    let line = identify_line();
    let mut seen = Seen::default();
    for home in &homes {
        if !all.active() {
            return None;
        }
        let answer = match control::ask(&*all.route, home, &line, all.pace.answer) {
            Ok(answer) => answer,
            // No socket, or nobody listening: no gateway to ask.
            Err(Silent::Absent) => continue,
            Err(Silent::TimedOut | Silent::Garbled | Silent::Unsafe(_)) => {
                seen.missed = true;
                continue;
            }
        };
        // Named once the answer came, as a follower names them.
        let (named, root) = (all.route.named(home), all.route.named(root));
        let instance = match identify(&answer, &named, &root) {
            Identified::Ready(instance) => instance,
            // Starting or draining: what it will serve is not said yet.
            Identified::Wait(_) => {
                seen.missed = true;
                continue;
            }
            // It said what it is, and it is no gateway to follow.
            Identified::Unsupported(_) | Identified::Failed(_) => continue,
        };
        // A home that is no profile of the root, as the gateway writes it,
        // was not verified as the profile the route takes it for.
        let profile = home.profile().clone();
        if !instance.serves(&profile) {
            continue;
        }
        seen.root = root.into_owned();
        seen.at.insert(profile.clone(), instance.id.clone());
        let found = seen.instances.entry(instance.id.clone());
        let via = BTreeMap::new();
        let found = found.or_insert_with(|| Found { instance, via });
        found.via.insert(profile, (answer, named.into_owned()));
    }
    Some(seen)
}
