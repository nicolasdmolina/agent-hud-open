//! Verifying a far machine: which machine the route leads to, read from
//! the machine itself before anything of it is followed.
use super::{
    follow::{reason, unsupported},
    Inner, Standing,
};
use crate::clients::hermes::{far::NoPicture, route::ssh::First};
use agent_hud_agents::hermes::gateway::MachineKey;

impl Inner {
    /// The key of the machine the route leads to, or how the machine
    /// stands instead. Every gateway found is asked for the install's id
    /// on a first socket of its own; all that answered must say the same,
    /// and at least one must have answered.
    pub(super) fn verified(&self) -> Result<MachineKey, Standing> {
        // No link: the standing says why from the link itself.
        let (link, number) = self.route.link().ok_or(Standing::Connecting)?;
        let picture = match self.route.far().discover(&link) {
            Ok(picture) => picture,
            Err(NoPicture::NotCovered(why)) => {
                return Err(Standing::NotCovered(reason(&why).to_owned()));
            }
            // The look did not finish: nothing is known, and it is taken
            // again.
            Err(NoPicture::Unreached(_)) => return Err(Standing::Connecting),
        };
        let mut lacking = unsupported(&picture.asked);
        let mut reported = Vec::new();
        for gateway in &picture.gateways {
            let Some((home, instance)) = gateway.sockets.first() else {
                continue;
            };
            if !self.active() {
                return Err(Standing::Connecting);
            }
            let profile = home.profile.name();
            match (self.route).first_socket(home, instance, self, self.pace.answer) {
                First::Reported(install) => reported.push((instance.id.clone(), install)),
                // It answers who it is and grants no ticket: an old build.
                First::Unsupported(why) => lacking.push((profile.to_owned(), why)),
                // Not heard: it says who it is on its first socket later,
                // and is followed only if it says the same.
                First::Failed(failure) => {
                    let line = format!("gateway not asked profile={profile} why={failure:?}");
                    self.log.line(&line);
                }
            }
        }
        let none_live = !lacking.is_empty();
        self.note_unfollowed(lacking);
        if reported.is_empty() && (none_live || picture.gateways.is_empty()) {
            let why = match none_live {
                true => "no_live_connection",
                false => "no_gateway_running",
            };
            return Err(Standing::NotCovered(why.to_owned()));
        }
        if reported.is_empty() {
            return Err(Standing::Connecting);
        }
        self.route
            .settle(&picture.root, &reported, number)
            .map_err(|why| Standing::NotCovered(why.reason_code().to_owned()))
    }
}
