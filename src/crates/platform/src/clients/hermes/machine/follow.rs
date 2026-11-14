//! Following a far machine once it is verified, and what ends that: the
//! observer stopping, or the route finding that it leads elsewhere.
use super::{Inner, Standing};
use crate::clients::hermes::{
    far::{NoPicture, NotCovered},
    followers::Followers,
    route::{ssh::Noticed, Route},
};
use agent_hud_agents::hermes::gateway::{
    far::{Asked, FarHome},
    Identified, Machine, MachineKey,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The least time between two looks a doubt asks for.
const RECHECK: Duration = Duration::from_secs(10);

impl Inner {
    /// Follows the gateways of the machine `key` names until the observer
    /// stops or the route says it leads there no more.
    pub(super) fn follow(&self, key: MachineKey) {
        let route: Arc<dyn Route> = self.route.clone();
        let machine = (Machine::Far(key.clone()), Some(self.label.clone()));
        let (requests, wake) = (self.requests.clone(), self.wake.clone());
        let Ok(all) = Followers::start(route, machine, &self.data, requests, wake, self.pace)
        else {
            return;
        };
        {
            let mut state = self.state();
            if !state.active {
                drop(state);
                return all.stop();
            }
            state.followers = Some(all.clone());
        }
        self.stand(Standing::Connected(key));
        let mut looked: Option<Instant> = None;
        let why = loop {
            match self.route.noticed(&|| !self.active()) {
                Noticed::Over => break None,
                Noticed::Uncovered(why) => break Some(why),
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
        let Some(why) = why else {
            return;
        };
        // Nothing of it stays: its cards are withdrawn, and it is
        // verified anew before anything is told again.
        if let Some(followers) = self.state().followers.take() {
            followers.stop();
        }
        self.route.forget();
        self.stand(Standing::NotCovered(why));
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
