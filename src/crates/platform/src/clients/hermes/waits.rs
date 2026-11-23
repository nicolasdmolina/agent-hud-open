//! How long one poll waits on the gateway: for the replies to what it
//! asks at once, and for a snapshot (ticket 72).
//!
//! A call may take `Pace::answer`, five seconds in the app. A poll goes
//! to the model whole, so one call that takes that long holds up every
//! card the poll found. Behind a quick poll that is within the bounds of
//! the design. A far gateway where nothing moves is polled every ten
//! seconds, and a card there is to show within thirteen: one call of
//! five seconds in the poll that finds a question, and its card is late.
//!
//! So what a poll asks last (the model's `order.rs`: the reads of
//! sessions nothing has changed in and the looks at idle ones, which no
//! new card waits on) is given `Pace::still` together, two seconds in
//! the app, from the moment the first of it is asked. What has not
//! answered by then is left as a call that was never answered is: a
//! look's session rests (`turns.rs`), a session's read goes to a lane
//! (`apart.rs`), its cards stay as they are, and the next poll asks
//! again. That is ten seconds, two, and the one second the design gives
//! a poll for its own work.
//!
//! Every other call keeps the long wait: a session that just asked is
//! read for as long as any read may take. A reading of one session
//! apart is no poll, and is never cut short.
use super::{visit::Visit, Model, Pace, Shared};
use agent_hud_agents::hermes::gateway::{Instance, Poll, Step};
use serde_json::Value;
use std::time::{Duration, Instant};

/// The waits of one poll, or of one reading of a session apart.
pub(super) struct Waits {
    /// Whether this is a poll, which gives what it asks last a short time.
    poll: bool,
    /// Until when what is asked last is waited for, once any of it is.
    cut: Option<Instant>,
}

impl Waits {
    pub(super) fn of(poll: bool) -> Self {
        Self { poll, cut: None }
    }

    /// Until when the replies to what `poll` asks now are waited for.
    /// None where it asks last and the time for that is up: nothing more
    /// is asked, and what is left is not known by this poll.
    pub(super) fn round(&mut self, pace: &Pace, poll: &Poll) -> Option<Instant> {
        let now = Instant::now();
        if !(self.poll && poll.last()) {
            return Some(now + pace.answer);
        }
        let cut = *self.cut.get_or_insert(now + pace.still);
        (now < cut).then_some(cut.min(now + pace.answer))
    }

    /// How long a snapshot may take to answer: twice what a call may,
    /// or, of a session that is asked last, what is left of that time.
    fn snapshot(&self, pace: &Pace) -> Duration {
        let whole = pace.answer * 2;
        let left = |cut: Instant| cut.saturating_duration_since(Instant::now());
        self.cut.map_or(whole, |cut| whole.min(left(cut)))
    }
}

/// Everything that waits in one session, from a visit that ends at once.
/// `still` is asked under the model's lock before anything subscribes.
pub(super) fn snapshot(
    shared: &Shared,
    instance: &Instance,
    step: &Step,
    waits: &Waits,
    still: &dyn Fn(&Model) -> bool,
) -> Option<Value> {
    let Step::Snapshot(session) = step else {
        return None;
    };
    let wait = waits.snapshot(&shared.pace);
    if wait.is_zero() {
        return None;
    }
    let (visit, told) = Visit::begin_while(shared, instance, session, wait, &|| true, still)?;
    visit.end();
    Some(told)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_agents::hermes::gateway::{
        machine_key, Gateway, GatewayId, Input, Machine, Profile, SESSION_AUTHORITY,
    };
    use serde_json::json;

    /// A poll of a made-up gateway of `machine`, which is ready.
    fn begun(machine: Machine) -> Poll {
        let mut gateway = Gateway::new(GatewayId::new(machine, Profile::Default));
        let answer = json!({"runtime_protocol": 1, "state": "ready", "instance_id": "one",
            "authority_epoch": 1, "api_origin": "http://127.0.0.1:54321",
            "capabilities": [SESSION_AUTHORITY],
            "served_profiles": [{"profile_id": "/synthetic", "home": "/synthetic"}]});
        let (answer, home) = (&answer, "/synthetic");
        gateway.apply(Input::Identified { answer, home }, 0);
        gateway.poll().expect("a gateway that is ready")
    }

    /// The same, with its list answered: one idle session. Of a far
    /// machine, all that is left to ask is the look at its stream.
    fn listed(machine: Machine) -> Poll {
        let mut poll = begun(machine);
        let lists = poll.steps();
        assert!(matches!(lists[..], [Step::List(_)]), "{lists:?}");
        let rows = json!({"sessions": [{"id": "s-idle", "status": "idle"}]});
        poll.answer(&lists[0], &rows);
        poll
    }

    fn far() -> Machine {
        let install = "0123456789abcdef0123456789abcdef";
        Machine::Far(machine_key(Some(install), "/Users/USER/.hermes").expect("a made-up identity"))
    }

    /// No test here waits anything out: the short time is a minute.
    fn pace() -> Pace {
        Pace {
            answer: Duration::from_secs(300),
            still: Duration::from_secs(60),
            ..Pace::default()
        }
    }

    #[test]
    fn what_a_poll_asks_last_is_waited_for_a_short_time_all_of_it_together() {
        let (pace, poll) = (pace(), listed(far()));
        assert!(matches!(poll.steps()[..], [Step::Watch { .. }]));
        let mut waits = Waits::of(true);
        // A snapshot taken before anything is asked last has its whole time.
        assert_eq!(waits.snapshot(&pace), pace.answer * 2);
        let began = Instant::now();
        let first = waits.round(&pace, &poll).expect("time for the looks");
        assert!(first >= began + pace.still);
        assert!(
            first <= Instant::now() + pace.still,
            "not a call's whole time"
        );
        // A second round of it gets what is left, not a time of its own,
        // and so does a snapshot of a session that is asked last.
        assert_eq!(waits.round(&pace, &poll), Some(first));
        assert!(waits.snapshot(&pace) <= pace.still);
        // Once the time is up nothing more of it is asked, and no
        // snapshot is taken.
        let (poll, cut) = (true, Some(Instant::now()));
        let mut over = Waits { poll, cut };
        assert_eq!(over.round(&pace, &listed(far())), None);
        assert!(over.snapshot(&pace).is_zero());
        // A call may never take longer for it than any call may.
        let brief = Pace {
            answer: Duration::from_secs(5),
            ..pace
        };
        let until = Waits::of(true).round(&brief, &listed(far()));
        assert!(until.expect("asked") <= Instant::now() + brief.answer);
    }

    #[test]
    fn every_other_call_has_as_long_as_a_call_may_take() {
        let pace = pace();
        let long = |until: Option<Instant>| {
            let left = until
                .expect("asked")
                .saturating_duration_since(Instant::now());
            left > pace.answer - Duration::from_secs(60)
        };
        // The lists of a far gateway's poll, which are not asked last.
        assert!(long(Waits::of(true).round(&pace, &begun(far()))));
        // A reading of one session apart, whatever it asks.
        let looks = listed(far());
        let mut apart = Waits::of(false);
        assert!(long(apart.round(&pace, &looks)));
        assert_eq!(apart.snapshot(&pace), pace.answer * 2);
        // A gateway of this machine looks at no idle session: its poll
        // is over with its list, and nothing of it was cut short.
        assert!(listed(Machine::This).steps().is_empty());
    }
}
