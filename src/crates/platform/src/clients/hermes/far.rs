//! Hermes on a far machine, over an open SSH link (`ssh_link`): one
//! exchange with a home's control socket, and finding the gateways there.
//! Both return plain data. Nothing here follows a gateway or opens a
//! WebSocket.
//!
//! **The exchange.** The far socket is never forwarded, and `nc -U` is not
//! used: neither proves who answers. Each exchange runs the relay
//! (`far/relay.py`) on the far machine. It is sent on standard input and
//! never written to that machine's disk. It checks, on open descriptors,
//! that the home, the socket or the pointer file and its folder are the
//! login user's own, and that the process listening runs as that user,
//! before it sends the one line. What that proves: the listener runs as
//! the login user. It cannot tell two processes of that user apart; same
//! user on the far machine is the trust boundary, as it is for Hermes's
//! own client. Where a check fails or cannot be made, the answer is the
//! named reason ([`Unchecked`]); nothing falls back to an unchecked path.
//!
//! **Deadlines.** The link's time limit ends this side's wait and nothing
//! more. The relay carries its own deadline and is gone when it passes. An
//! exchange that was cut off counts as running until then: no second one
//! is started with that home before ([`Silent::Busy`]).
//!
//! A ticket is in an answer and nowhere else: never in an error, and
//! [`Answered`] does not print.
use crate::ssh_link::{End, Failure, FarCommand, Link};
use agent_hud_agents::hermes::gateway::far::{
    homes, picture, relay_said, Answered, Asked, FarHome, Homes, NoHomes, Picture, Quiet, Request,
    Unchecked, HOMES_SCRIPT, HOMES_SECONDS, MOST_ANSWER, RELAY_SECONDS,
};
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

#[cfg(all(test, unix))]
mod tests;

/// The relay, as it is sent.
const RELAY: &str = include_str!("far/relay.py");
/// `python3` reads the relay from standard input. It runs apart from the
/// user's own Python settings and packages, and writes no bytecode.
const RUN_RELAY: &str = "exec python3 -I -S -B - \"$@\"";
/// How much longer this side waits than the far deadline.
const MARGIN: Duration = Duration::from_secs(2);
/// The most the homes command may print.
const MOST_HOMES: usize = 256 * 1024;

/// Why an exchange brought no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silent {
    /// The relay ran and says why ([`Quiet`]).
    Relay(Quiet),
    /// This side stopped waiting. The relay may still run there, until its
    /// deadline.
    CutOff,
    /// An exchange with this home was cut off and its deadline has not
    /// passed. Nothing was started.
    Busy,
    /// The link refused the command, or is lost.
    Link(Failure),
}

/// One exchange, and how long it took over the open link.
#[derive(Debug)]
pub struct Exchanged {
    pub said: Result<Answered, Silent>,
    pub took: Duration,
    /// Two where an empty reply was asked again.
    pub tries: u8,
}

/// Why a far machine gave no picture of its gateways.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoPicture {
    /// The machine is not covered, and this is the reason.
    NotCovered(NotCovered),
    /// The discovery did not finish. Nothing is known of the machine, and
    /// nothing about it changes.
    Unreached(Silent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotCovered {
    /// No Hermes home on that machine.
    NoHermes,
    /// A check failed or could not be made: no `python3`, a home others
    /// may write, a listener of another user. `home` is the far path it
    /// was made for, or None for the whole machine.
    Unchecked {
        why: Unchecked,
        home: Option<String>,
    },
    /// The root home it named is not a canonical path.
    RootNotCanonical,
}

/// The exchanges with one far machine. It remembers which homes have a
/// relay that was cut off and may still run.
pub struct Far {
    seconds: u32,
    program: String,
    /// Each home, and the homes command under an empty name, with when a
    /// cut-off program there is surely gone.
    busy: Mutex<HashMap<String, Instant>>,
}

impl Default for Far {
    fn default() -> Self {
        Self::new(RELAY_SECONDS)
    }
}

impl Far {
    /// `seconds` is the relay's own deadline on the far machine.
    pub fn new(seconds: u32) -> Self {
        Self::with_program(seconds, RELAY.to_owned())
    }

    fn with_program(seconds: u32, program: String) -> Self {
        Self {
            seconds,
            program,
            busy: Mutex::new(HashMap::new()),
        }
    }

    /// Whether a program cut off under `name` may still run, and if not,
    /// forgets it.
    fn busy(&self, name: &str) -> bool {
        let mut busy = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        let still = busy.get(name).is_some_and(|until| Instant::now() < *until);
        if !still {
            busy.remove(name);
        }
        still
    }

    /// A program under `name` was cut off just now. Whatever of it runs
    /// started before this moment, so it is gone `seconds` from now.
    fn cut_off(&self, name: &str, seconds: u32) {
        let until = Instant::now() + Duration::from_secs(seconds.into()) + MARGIN;
        let mut busy = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        busy.insert(name.to_owned(), until);
    }

    /// One exchange with the control socket of the home at `home`, a
    /// canonical path of the far machine. An empty reply is asked once
    /// more. Only `identify` and `session-ticket` can be asked.
    pub fn exchange(&self, link: &Link, home: &str, request: Request) -> Exchanged {
        let started = Instant::now();
        let line = request.line();
        let mut said = self.relay(link, home, &line);
        let mut tries = 1;
        if said == Err(Silent::Relay(Quiet::Empty)) {
            said = self.relay(link, home, &line);
            tries = 2;
        }
        Exchanged {
            said,
            took: started.elapsed(),
            tries,
        }
    }

    /// Runs the relay once for `home` with `line`.
    fn relay(&self, link: &Link, home: &str, line: &str) -> Result<Answered, Silent> {
        if self.busy(home) {
            return Err(Silent::Busy);
        }
        let seconds = self.seconds.to_string();
        let ran = link.run(&FarCommand {
            script: RUN_RELAY,
            args: &[&seconds, home, line],
            stdin: Some(self.program.as_bytes()),
            limit: Duration::from_secs(self.seconds.into()) + MARGIN,
            most_output: MOST_ANSWER + 64,
        });
        match ran {
            Ok(ran) => match ran.end {
                End::Ended(exit) => relay_said(exit, &ran.stdout).map_err(Silent::Relay),
                End::CutOff(_) => {
                    self.cut_off(home, self.seconds);
                    Err(Silent::CutOff)
                }
            },
            // A link that was lost under the relay says nothing of it.
            Err(failure @ Failure::Lost(_)) => {
                self.cut_off(home, self.seconds);
                Err(Silent::Link(failure))
            }
            Err(failure) => Err(Silent::Link(failure)),
        }
    }

    /// The Hermes homes of the far machine: its root, as the login shell
    /// there has it, and each profile's folder, all canonical far paths.
    pub fn homes(&self, link: &Link) -> Result<Homes, NoPicture> {
        if self.busy("") {
            return Err(NoPicture::Unreached(Silent::Busy));
        }
        let ran = link.run(&FarCommand {
            script: HOMES_SCRIPT,
            args: &[],
            stdin: None,
            limit: Duration::from_secs(HOMES_SECONDS.into()) + MARGIN,
            most_output: MOST_HOMES,
        });
        let unreached = |silent| Err(NoPicture::Unreached(silent));
        let ran = match ran {
            Ok(ran) => ran,
            Err(failure @ Failure::Lost(_)) => {
                self.cut_off("", HOMES_SECONDS);
                return unreached(Silent::Link(failure));
            }
            Err(failure) => return unreached(Silent::Link(failure)),
        };
        if ran.end != End::Ended(0) {
            if matches!(ran.end, End::CutOff(_)) {
                self.cut_off("", HOMES_SECONDS);
                return unreached(Silent::CutOff);
            }
            return unreached(Silent::Relay(Quiet::Failed));
        }
        let not_covered = |why| Err(NoPicture::NotCovered(why));
        match homes(&ran.stdout) {
            Ok(homes) => Ok(homes),
            Err(NoHomes::NoHermes) => not_covered(NotCovered::NoHermes),
            Err(NoHomes::RootNotCanonical) => not_covered(NotCovered::RootNotCanonical),
            Err(NoHomes::NoPython) => not_covered(NotCovered::Unchecked {
                why: Unchecked::NoPython,
                home: None,
            }),
            Err(NoHomes::Garbled) => unreached(Silent::Relay(Quiet::Garbled)),
        }
    }

    /// Finds the gateways of the far machine: its homes, `identify` asked
    /// of every one, and the answers grouped by instance. A machine where
    /// the relay cannot run, or where a check fails for any home, is not
    /// covered and says why. A link lost on the way gives no picture.
    pub fn discover(&self, link: &Link) -> Result<Picture<Silent>, NoPicture> {
        let found = self.homes(link)?;
        self.identify_all(link, &found, found.homes.iter())
    }

    /// Asks `identify` of `order`, which is every home of `found` once.
    fn identify_all<'a>(
        &self,
        link: &Link,
        found: &Homes,
        order: impl Iterator<Item = &'a FarHome>,
    ) -> Result<Picture<Silent>, NoPicture> {
        let mut answers = Vec::new();
        for home in order {
            let said = self.exchange(link, &home.path, Request::Identify).said;
            if let Err(Silent::Link(Failure::Lost(_))) = said {
                return Err(NoPicture::Unreached(said.unwrap_err()));
            }
            answers.push((home.clone(), said));
        }
        let picture = picture(found, answers);
        // The first home, in the homes' own order, that failed a check.
        for (home, asked) in &picture.asked {
            if let Asked::Quiet(Silent::Relay(Quiet::Unchecked(why))) = asked {
                return Err(NoPicture::NotCovered(NotCovered::Unchecked {
                    why: *why,
                    home: Some(home.path.clone()),
                }));
            }
        }
        Ok(picture)
    }
}
