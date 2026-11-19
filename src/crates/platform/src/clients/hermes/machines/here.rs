//! How this machine stands in the machines list (ticket 69): from what its
//! own followers know, as a far machine's row goes by its route. It is
//! read on the thread that looks at the list, with nothing of the book
//! locked, and kept in the book until the next look.
use super::{book::Book, words};
use crate::clients::hermes::{control::Silent, followers::Followers};
use agent_hud_agents::{
    hermes::gateway::{Connection, Failure},
    MachineState,
};
use std::sync::atomic::Ordering;

/// How this machine stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::clients::hermes) enum Here {
    /// A gateway here is followed.
    Followed,
    /// Not known yet: before the first look, while a gateway starts, or
    /// where a home here could not be asked.
    #[default]
    Connecting,
    /// The only gateways here lack the live connection.
    NoLiveConnection,
    /// Hermes is here and no gateway runs.
    NoGateway,
    /// A gateway here refused or failed in another way.
    Unreachable,
    /// There is no Hermes home here at all.
    NoHermes,
}

/// What one follower's gateway was when its last dial settled, as far as
/// how the machine stands goes. Identify answering "ready" is not yet
/// followed: an old build answers so at every dial and then refuses the
/// ticket, so a dial is read only once it settled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::clients::hermes) enum Gateway {
    /// A socket on it is open.
    Followed,
    /// Not dialled yet, or starting or draining.
    #[default]
    Starting,
    /// It lacks the live connection.
    Old,
    /// Nobody listens at its control socket.
    Stopped,
    Failed,
}

impl Gateway {
    pub(in crate::clients::hermes) fn of(connection: &Connection, open: bool) -> Self {
        match connection {
            Connection::Ready(_) if open => Self::Followed,
            Connection::Ready(_) | Connection::Disconnected | Connection::Connecting => {
                Self::Starting
            }
            Connection::Unsupported(_) => Self::Old,
            Connection::Failed(Failure::Unreachable) => Self::Stopped,
            Connection::Failed(_) => Self::Failed,
        }
    }
}

/// How the machine stands, from whether its Hermes home is there, what
/// each follower's gateway is, and whether the last look had a doubt.
pub(super) fn standing(home: bool, gateways: &[Gateway], doubt: bool) -> Here {
    let any = |kind: Gateway| gateways.contains(&kind);
    if !home {
        Here::NoHermes
    } else if any(Gateway::Followed) {
        Here::Followed
    } else if any(Gateway::Starting) {
        Here::Connecting
    } else if any(Gateway::Old) {
        Here::NoLiveConnection
    } else if doubt {
        Here::Connecting
    } else if any(Gateway::Failed) {
        Here::Unreachable
    } else {
        Here::NoGateway
    }
}

/// How this machine stands now, from its followers.
pub(super) fn read(local: &Followers) -> Here {
    let home = !matches!(local.route.inventory(), Err(Silent::Absent));
    let gateways: Vec<Gateway> = local.set().iter().map(|one| one.settled()).collect();
    standing(home, &gateways, local.doubt.load(Ordering::Acquire))
}

impl Here {
    /// The state and the note its row says.
    pub(super) fn told(self) -> (MachineState, Option<String>) {
        match self {
            Self::Followed => (MachineState::Connected, None),
            Self::Connecting => (MachineState::Connecting, None),
            Self::NoLiveConnection => (
                MachineState::NotCovered,
                Some(words::here_no_live_connection()),
            ),
            Self::NoGateway => (MachineState::CannotReach, Some(words::here_no_gateway())),
            Self::Unreachable => (MachineState::CannotReach, Some(words::here_unreachable())),
            Self::NoHermes => (MachineState::NotCovered, Some(words::here_no_hermes())),
        }
    }
}

impl Book {
    /// How this machine stands from now on. Told with the list, after.
    pub(super) fn here_is(&self, here: Here) {
        let mut state = self.state();
        if std::mem::replace(&mut state.here, here) != here {
            self.log.line(&format!("this machine stands={here:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{standing, Gateway::*, Here};

    #[test]
    fn this_machine_stands_by_what_its_followers_know() {
        let cases = [
            (false, vec![Followed], false, Here::NoHermes),
            (false, vec![], false, Here::NoHermes),
            (true, vec![Stopped, Followed], false, Here::Followed),
            (true, vec![Followed, Old], true, Here::Followed),
            (true, vec![Stopped, Starting], false, Here::Connecting),
            (true, vec![Starting, Old], false, Here::Connecting),
            (true, vec![Stopped, Old], false, Here::NoLiveConnection),
            (true, vec![Old], true, Here::NoLiveConnection),
            (true, vec![Stopped], true, Here::Connecting),
            (true, vec![Failed], true, Here::Connecting),
            (true, vec![Stopped, Failed], false, Here::Unreachable),
            (true, vec![Stopped], false, Here::NoGateway),
            (true, vec![], false, Here::NoGateway),
        ];
        for (home, gateways, doubt, expected) in cases {
            let found = standing(home, &gateways, doubt);
            assert_eq!(found, expected, "{home} {gateways:?} {doubt}");
        }
    }

    /// Only an open socket is followed: identify's "ready" alone is not.
    #[test]
    fn a_gateway_is_followed_only_while_its_socket_is_open() {
        use agent_hud_agents::hermes::gateway::{Connection, Failure, Instance, Unsupported};
        let ready = Connection::Ready(Instance {
            id: "instance".into(),
            authority_epoch: 1,
            origin: "http://127.0.0.1:1".into(),
            port: 1,
            profile_id: "default".into(),
            served: vec![],
            draining: false,
            version: None,
        });
        let cases = [
            (ready.clone(), true, Followed),
            (ready, false, Starting),
            (Connection::Disconnected, false, Starting),
            (Connection::Connecting, false, Starting),
            (Connection::Unsupported(Unsupported::TicketVerb), false, Old),
            (Connection::Failed(Failure::Unreachable), false, Stopped),
            (Connection::Failed(Failure::TicketRefused), false, Failed),
        ];
        for (connection, open, expected) in cases {
            assert_eq!(super::Gateway::of(&connection, open), expected, "{connection:?}");
        }
    }

    /// Before the first look, the row is not said to be anything yet.
    #[test]
    fn this_machine_is_connecting_before_the_first_look() {
        assert_eq!(Here::default(), Here::Connecting);
    }

    /// Only a followed gateway is connected; no Hermes home is no failure.
    #[test]
    fn each_standing_says_its_state() {
        use agent_hud_agents::MachineState as State;
        let states = [
            (Here::Followed, State::Connected, false),
            (Here::Connecting, State::Connecting, false),
            (Here::NoLiveConnection, State::NotCovered, true),
            (Here::NoGateway, State::CannotReach, true),
            (Here::Unreachable, State::CannotReach, true),
            (Here::NoHermes, State::NotCovered, true),
        ];
        for (here, state, noted) in states {
            let (told, note) = here.told();
            assert_eq!((told, note.is_some()), (state, noted), "{here:?}");
        }
    }
}
