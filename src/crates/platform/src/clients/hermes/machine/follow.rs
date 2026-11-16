//! What a route does once it is verified: as the owner of its machine it
//! watches for word that it leads there no more, and as an alias it
//! follows nothing and waits to be asked.
use super::{Inner, Role, Standing, State};
use crate::clients::hermes::{
    far::{NoPicture, NotCovered},
    route::{ssh::Noticed, Stop},
};
use agent_hud_agents::hermes::gateway::{
    far::{Asked, FarHome},
    Identified, MachineKey,
};
use std::{
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};

/// The least time between two looks a doubt asks for.
const RECHECK: Duration = Duration::from_secs(10);
/// The first wait before verifying again, in the waits between two dials
/// of a gateway (`Pace::retry`): half a second. Doubled each time.
const SOON: u32 = 2;
/// The longest wait before verifying again, in the longest waits between
/// two dials (`Pace::retry_limit`): half a minute.
const LATER: u32 = 6;

/// What verifying waits on beside its own work: the route stopping. Not
/// being asked to look again, which waits for the look under way.
impl Stop for Inner {
    fn wait(&self, time: Duration) -> bool {
        let state = self.state();
        let waits = |state: &mut State| state.active;
        let (state, _) = (self.stopping.wait_timeout_while(state, time, waits))
            .unwrap_or_else(PoisonError::into_inner);
        !state.active
    }
}

impl Inner {
    pub(super) fn work(self: Arc<Self>) {
        let (soon, later) = (self.pace.retry * SOON, self.pace.retry_limit * LATER);
        let mut wait = soon;
        while self.active() {
            let looked = self.verified();
            // Whoever asked for a look meanwhile has this one. The keeper
            // is held only to be told: a route outlives nothing by it.
            self.state().stirred = false;
            let Some(keeper) = self.keeper() else {
                return;
            };
            let again = match looked {
                Ok(key) => {
                    wait = soon;
                    let role = keeper.verified(&self, &key);
                    drop(keeper);
                    match role {
                        Role::Owner => self.own(key),
                        Role::Alias => self.idle(key),
                        Role::Over => return,
                    }
                }
                // Not followed: nothing made to ask it is kept.
                Err(standing) => {
                    self.route.forget();
                    self.stand(standing.clone());
                    keeper.unverified(&self, &standing);
                    false
                }
            };
            if !again {
                self.pause(wait);
                wait = (wait * 2).min(later);
            }
        }
    }

    /// Owns the machine `key` names until the route stops, is asked to
    /// look again (another route owns the machine now), or finds that it
    /// leads there no more, which it tells. Whether to verify at once.
    fn own(&self, key: MachineKey) -> bool {
        self.stand(Standing::Connected(key.clone()));
        let mut looked: Option<Instant> = None;
        let why = loop {
            match (self.route).noticed(&|| !self.active() || self.stirred()) {
                // What it made for the gateways is the next owner's to
                // make again, over its own link.
                Noticed::Over => {
                    self.route.forget();
                    return self.active();
                }
                Noticed::Uncovered(why) => break why,
                // A check failed for one home: the machine is uncovered
                // where a look at all of it says so too.
                Noticed::Doubt => {
                    if looked.is_some_and(|at| at.elapsed() < RECHECK) {
                        self.pause(RECHECK);
                    }
                    looked = Some(Instant::now());
                    if let Some(why) = self.uncovered() {
                        self.route.uncover(&why);
                    }
                }
            }
        };
        // It leads to that machine no more: said first, so that nothing
        // more of the machine is taken over this route, then verified
        // anew before anything is told again.
        if let Some(keeper) = self.keeper() {
            keeper.left(self, &key);
        }
        self.route.forget();
        self.stand(Standing::NotCovered(why));
        false
    }

    /// The machine `key` names is followed over another route, or is the
    /// one Agent HUD runs on: nothing is followed here, and what
    /// verifying made is let go. Waits until it is asked to verify again.
    fn idle(&self, key: MachineKey) -> bool {
        self.route.forget();
        self.stand(Standing::Alias(key));
        let state = self.state();
        let waits = |state: &mut State| state.active && !state.stirred;
        let state =
            (self.stopping.wait_while(state, waits)).unwrap_or_else(PoisonError::into_inner);
        state.active
    }

    /// Why the machine is not one to follow, as a look at all of it says
    /// now. None where it is, or where the look did not finish.
    fn uncovered(&self) -> Option<String> {
        let (link, _) = self.route.link()?;
        match self.route.far().discover(&link) {
            Err(NoPicture::NotCovered(why)) => Some(reason(&why).to_owned()),
            _ => None,
        }
    }
}

/// A word for why a far machine is not covered.
pub(super) fn reason(why: &NotCovered) -> &'static str {
    match why {
        NotCovered::NoHermes => "no_hermes_home",
        NotCovered::Unchecked { why, .. } => why.reason_code(),
        NotCovered::RootNotCanonical => "root_home_not_canonical",
        NotCovered::TooManyProfiles => "too_many_profiles",
    }
}

/// The gateways of `asked` that lack the live connection.
pub(super) fn unsupported<Q>(asked: &[(FarHome, Asked<Q>)]) -> Vec<(String, &'static str)> {
    let lacking = |(home, asked): &(FarHome, Asked<Q>)| match asked {
        Asked::Answered(Identified::Unsupported(why)) => {
            Some((home.profile.name().to_owned(), why.reason_code()))
        }
        _ => None,
    };
    asked.iter().filter_map(lacking).collect()
}
