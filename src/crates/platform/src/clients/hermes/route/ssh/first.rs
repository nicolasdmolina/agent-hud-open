//! The first socket to a far gateway, before anything follows it: the
//! forward, then the ticket, then the dial, as every socket is opened,
//! and one call on it, for the install's own id. It is how a route learns
//! which machine it leads to, so nothing of the gateway is told to
//! anyone here: the socket is closed once it has answered.
use super::{
    super::{Home, Route, Stop},
    Ssh,
};
use crate::clients::hermes::{control, link::silent, socket::Socket};
use agent_hud_agents::hermes::gateway::{
    far::FarHome, granted, profiles_list, reported_install, ticket_line, Denied, Failure, Instance,
    Profile, Scope,
};
use std::{path::PathBuf, time::Duration};

/// What came of a first socket.
pub(crate) enum First {
    /// The install id the gateway reports, as it wrote it; None where it
    /// names none.
    Reported(Option<String>),
    /// Its control socket grants no ticket: a build without the live
    /// connection, by Hermes's own reason.
    Unsupported(&'static str),
    /// It could not be asked this time.
    Failed(Failure),
}

impl Ssh {
    /// The install id the gateway `instance` reports, asked on a socket
    /// opened with a ticket from the control socket of `home`, which is
    /// the home that answered as that instance. None where it names none.
    pub(crate) fn first_socket(
        &self,
        home: &FarHome,
        instance: &Instance,
        stop: &dyn Stop,
        wait: Duration,
    ) -> First {
        let grant = match self.ticket(home, instance, stop, wait) {
            Ok(grant) => grant,
            Err(failure) => return First::Failed(failure),
        };
        let ticket = match granted(&grant, instance) {
            Ok(ticket) => ticket,
            Err(Denied::Unsupported(why)) => return First::Unsupported(why.reason_code()),
            Err(Denied::Failed(failure)) => return First::Failed(failure),
        };
        drop(grant);
        let asked = || -> Result<Option<String>, Failure> {
            let mut socket = Socket::dial(self, instance)?.upgrade(ticket)?;
            let profile = Profile::of_home(&home.path);
            let told = socket.call(&profiles_list(&profile, Scope::Host), wait);
            socket.close();
            Ok(reported_install(&told?).map(str::to_owned))
        };
        match asked() {
            Ok(install) => First::Reported(install),
            Err(failure) => First::Failed(failure),
        }
    }

    /// The control socket's answer to a ticket for `instance`, asked once
    /// the forward to it is in place.
    fn ticket(
        &self,
        home: &FarHome,
        instance: &Instance,
        stop: &dyn Stop,
        wait: Duration,
    ) -> Result<serde_json::Value, Failure> {
        let stopped = |stop: &dyn Stop| match stop.wait(Duration::ZERO) {
            true => Err(Failure::Unreachable),
            false => Ok(()),
        };
        self.prepare(instance, stop)?;
        stopped(stop)?;
        let path = PathBuf::from(&home.path);
        let asked = Home::at(path.clone(), path, home.profile.clone());
        let line = ticket_line(instance, Scope::Host);
        let grant = control::ask(self, &asked, &line, wait).map_err(silent)?;
        stopped(stop)?;
        Ok(grant)
    }
}
