//! Reads a far machine's Hermes history over its SSH link: runs the reader
//! program (`agents::hermes::far_history`) there once and returns what it
//! answered, checked whole. One read at a time for each machine. Nothing
//! here schedules a read or stores one.
//!
//! The program is sent on standard input and is never a file on the far
//! disk. It opens that machine's own `state.db` so that it cannot change
//! it, holds what it read in memory, closes the database, and only then
//! answers, so a slow link never keeps a reader open there.
//!
//! **An answer is used whole or not at all.** The far program's exit
//! status, anything on its standard error, its last line and its counts,
//! and the shape of every row are checked on every read. What fails is
//! logged with the exit status and never with a row's text.
//!
//! **A local time limit does not stop the far reader**, so the machine's
//! slot (`slot.rs`) is freed only when the reader is known to be gone:
//! its answer ended with an exit status, or its own deadline has passed
//! since its `started` line was seen here. A lost link, a reconnect and a
//! change of route free nothing. The far side holds the same rule by
//! itself: a reader that finds another one of the same machine alive
//! opens nothing, whichever of the machine's homes either one reads.
use crate::ssh_link::{self, End, FarCommand, Link, Refused};
use agent_hud_agents::hermes::far_history::{self as far, First, No, Reply, Request};
use std::{
    thread,
    time::{Duration, Instant},
};

mod batches;
mod failure;
mod slot;

pub use batches::{Sessions, Skip};
pub use failure::Failure;
pub use slot::{Hold, Slot, Slots};

/// The first and last lines of an answer, beside the bound on its rows.
const BESIDE_ROWS: usize = 4 * 1024;

#[derive(Clone, Debug)]
pub struct Settings {
    /// The reader's own deadline, which it counts from its `started`.
    pub deadline: Duration,
    /// Added to the deadline before a cut-off reader is taken to be gone.
    pub margin: Duration,
    /// How long this side waits for an answer: a little longer than the
    /// deadline, for the reader's start.
    pub local_wait: Duration,
    /// The most bytes of rows one read returns.
    pub most_output: usize,
    /// The most sessions asked for in one read.
    pub batch: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(20),
            margin: Duration::from_secs(2),
            local_wait: Duration::from_secs(25),
            most_output: far::MOST_OUTPUT,
            batch: 25,
        }
    }
}

/// Runs reads. One for the app; the slot it is given says which machine.
pub struct Reader {
    settings: Settings,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

impl Reader {
    /// `log` is given one line for each read and each wait. A line holds
    /// kinds, counts, times and statuses, and never a row's text.
    pub fn new(settings: Settings, log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            settings,
            log: Box::new(log),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn say(&self, line: String) {
        (self.log)(&line);
    }

    /// One read of the Hermes home `home`, a canonical far path, on the
    /// machine whose slot is `slot`. It waits its turn: for a read under
    /// way, and for a reader that was cut off and may still run. Where
    /// nothing is known of a reader there, it probes first. `Reply::No`
    /// and `Reply::Failed` come back as a [`Failure`].
    pub fn read(
        &self,
        link: &Link,
        slot: &Slot,
        home: &str,
        request: &Request,
    ) -> Result<Reply, Failure> {
        let kind = request.kind();
        if !request.askable() {
            // A page that starts below rowid zero is never sent.
            self.say(format!("history {kind} not asked: a rowid below zero"));
            return Err(Failure::Refused(No::Request));
        }
        let _turn = slot.turn();
        for _ in 0..3 {
            match slot.hold() {
                Hold::Free => return self.run(link, slot, home, request),
                Hold::Until(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if !left.is_zero() {
                        self.say(format!(
                            "history {kind} waits {} ms: a reader cut off after it started may run until its deadline",
                            left.as_millis()
                        ));
                        thread::sleep(left);
                    }
                    slot.lapsed(until);
                }
                Hold::StartUnknown => {
                    self.say(format!(
                        "history {kind} probes first: nothing here says whether a reader runs there"
                    ));
                    let probed = self.run(link, slot, home, &Request::Probe)?;
                    if *request == Request::Probe {
                        return Ok(probed);
                    }
                }
            }
        }
        Err(Failure::Busy)
    }

    /// Starts one reader and takes its answer. The slot is left saying
    /// what is now known of that reader.
    fn run(
        &self,
        link: &Link,
        slot: &Slot,
        home: &str,
        request: &Request,
    ) -> Result<Reply, Failure> {
        let kind = request.kind();
        let settings = &self.settings;
        let args = far::arguments(
            slot.machine(),
            home,
            settings.deadline,
            settings.most_output,
            request,
        );
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let command = FarCommand {
            script: far::SCRIPT,
            args: &args,
            stdin: Some(far::PROGRAM.as_bytes()),
            limit: settings.local_wait,
            most_output: settings.most_output + BESIDE_ROWS,
        };
        let (round, before) = slot.begin();
        let held = settings.deadline + settings.margin;
        let began = Instant::now();
        let mut seen = false;
        let ran = link.run_watched(&command, &mut |stdout| {
            if !seen && far::first_line(stdout) == First::Started {
                seen = true;
                slot.started(round, Instant::now() + held);
            }
        });
        let ms = began.elapsed().as_millis();
        let ran = match ran {
            Ok(ran) => ran,
            Err(failure) => {
                // These three are refused before `ssh` is run, so no
                // reader was started and the slot is as it was.
                if let ssh_link::Failure::Refused(
                    Refused::Closed | Refused::CommandTooLong | Refused::HereDocument,
                ) = failure
                {
                    slot.restore(round, before);
                }
                self.say(format!("history {kind} not run: {failure}"));
                return Err(Failure::Link(failure));
            }
        };
        let started = if seen { "seen" } else { "not seen" };
        let exit = match ran.end {
            // 255 is `ssh`'s own failure as well as a far status, so it
            // does not show the far program gone.
            End::Ended(status) if status != 255 => status,
            End::Ended(status) => {
                self.say(format!(
                    "history {kind} failed exit={status} after {ms} ms, started {started}"
                ));
                return Err(Failure::Exit(status));
            }
            End::CutOff(cut) => {
                let failure = Failure::CutOff(cut);
                self.say(format!(
                    "history {kind} {failure} after {ms} ms, started {started}"
                ));
                return Err(failure);
            }
        };
        // The far program ended and its status came back: it is gone.
        let read = matches!(request, Request::Prints { .. } | Request::Sessions { .. });
        let answer = if exit == far::NO_PYTHON {
            Err(Failure::NoHistory)
        } else if exit == far::TOO_MUCH_MEMORY && read {
            // It ended itself over a value too long to hold: too large.
            Ok(Reply::TooLarge)
        } else if exit != 0 {
            Err(Failure::Exit(exit))
        } else if !ran.stderr.is_empty() {
            Err(Failure::Stderr)
        } else {
            match far::parse(request, &ran.stdout) {
                Err(rejected) => Err(Failure::Rejected(rejected)),
                Ok(Reply::No(No::Busy)) => Err(Failure::Busy),
                Ok(Reply::No(No::Sqlite)) => Err(Failure::NoHistory),
                Ok(Reply::No(no)) => Err(Failure::Refused(no)),
                Ok(Reply::Failed(far)) => Err(Failure::Far(far)),
                Ok(reply) => Ok(reply),
            }
        };
        if answer == Err(Failure::Busy) {
            slot.busy(round);
        } else {
            slot.ended(round);
        }
        // The far program's own word for a crash is one line that names
        // the kind of error. Any other text is counted and dropped.
        let crash = ran.stderr.trim_end().strip_prefix("far-history: crash ");
        let crash = crash.filter(|kind| kind.chars().all(|c| c.is_ascii_alphanumeric()));
        let crash = crash
            .map(|kind| format!(" crash={kind}"))
            .unwrap_or_default();
        let bytes = ran.stdout.len();
        match &answer {
            Ok(reply) => self.say(format!(
                "history {kind} ok exit={exit} {ms} ms {bytes} B{}",
                batches::counts(reply)
            )),
            Err(failure) => self.say(format!(
                "history {kind} failed exit={exit} stderr={} B{crash} {ms} ms: {failure}",
                ran.stderr.len()
            )),
        }
        answer
    }
}

#[cfg(all(test, unix))]
#[path = "far_history/unix/tests/mod.rs"]
mod tests;
