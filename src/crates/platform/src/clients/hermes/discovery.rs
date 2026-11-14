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
//! still claim what it claimed, so such a look settles no contest. A
//! home with no socket misses nothing by itself; a gateway that was
//! followed and was not heard keeps its claims all the same (`settle.rs`).
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
use std::{collections::BTreeMap, sync::Arc};

/// What one look found. A home is told of by its place on the machine
/// (`Home::profile`): the root home, or `profiles/<name>`.
#[derive(Default)]
pub(super) struct Seen {
    /// The root home of the machine, as a gateway writes it.
    pub(super) root: String,
    /// Every instance that answered, by its id.
    pub(super) instances: BTreeMap<String, Found>,
    /// Which instance answered at each home.
    pub(super) at: BTreeMap<Profile, String>,
    /// A home that has a gateway to ask gave no answer to go by: this
    /// look does not know every claim there is.
    pub(super) missed: bool,
    /// Which look this is, counted from the first.
    pub(super) number: u64,
}

/// One gateway, as it answered.
pub(super) struct Found {
    pub(super) instance: Instance,
    /// Each home that answered as this instance: what it answered there,
    /// and the home as the gateway writes it.
    pub(super) via: BTreeMap<Profile, (Value, String)>,
}

impl Seen {
    /// The profile the home at `place` is, as its gateway and every id of
    /// its sessions name it: by the home's path alone, as Hermes does. A
    /// named profile's is its place. The root's is the default profile,
    /// or, for a root that lies at `…/profiles/<name>`, that name.
    pub(super) fn named(&self, place: &Profile) -> Profile {
        match place {
            Profile::Default => Profile::of_home(&self.root),
            named => named.clone(),
        }
    }
}

/// Looks at the machine for as long as the observer runs.
pub(super) fn work(all: Arc<Followers>) {
    let by_clock = all.pace.discover.is_some();
    while all.active() {
        // By the clock, the first look comes at once and each wait below
        // ends in one. Without it, only a look that was asked for.
        if all.look.take() || by_clock {
            let number = all.look.begin();
            if let Some(mut seen) = look(&all) {
                seen.number = number;
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
            // No socket, or nobody listening. Whether that leaves a claim
            // open is for whoever knows who answered there before.
            Err(Silent::Absent) => continue,
            Err(Silent::TimedOut | Silent::Garbled | Silent::Unsafe(_) | Silent::Unclear) => {
                seen.missed = true;
                continue;
            }
        };
        // Named once the answer came, as a follower names it.
        let named = all.route.named(home);
        let instance = match identify(&answer, &named) {
            Identified::Ready(instance) => instance,
            // Starting or draining: what it will serve is not said yet.
            Identified::Wait(_) => {
                seen.missed = true;
                continue;
            }
            // It said what it is, and it is no gateway to follow.
            Identified::Unsupported(_) | Identified::Failed(_) => continue,
        };
        // A profile's folder that leads to a home of another name is not
        // the profile the route takes it for, and is left alone.
        let place = home.profile().clone();
        if place != Profile::Default && Profile::of_home(&*named) != place {
            continue;
        }
        seen.at.insert(place.clone(), instance.id.clone());
        let found = seen.instances.entry(instance.id.clone());
        let via = BTreeMap::new();
        let found = found.or_insert_with(|| Found { instance, via });
        found.via.insert(place, (answer, named.into_owned()));
    }
    seen.root = all.route.named(root).into_owned();
    Some(seen)
}
