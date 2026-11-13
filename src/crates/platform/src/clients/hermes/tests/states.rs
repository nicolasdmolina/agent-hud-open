//! What the connection tells core each live session is doing: every input
//! goes through `tell()`, which hands the model's `live_states()` to
//! `Requests::set_native_states`, and an observer that stops clears them.
use super::{fake::Fake, fake::PROFILE, lab, until, Lab};
use agent_hud_agents::{hermes::AGENT, LiveState, SOLE_READER};

const SESSION: &str = "local-states";

/// What core holds for Hermes, sorted.
fn told(lab: &Lab) -> Vec<(String, LiveState)> {
    let all = lab.requests.native_states();
    let mut sessions: Vec<_> = all
        .of(AGENT.id, SOLE_READER)
        .into_iter()
        .flat_map(|states| states.sessions.clone())
        .collect();
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    sessions
}

fn read_ms(lab: &Lab) -> i64 {
    let all = lab.requests.native_states();
    let reading = all.of(AGENT.id, SOLE_READER);
    reading.map_or(0, |states| states.read_ms)
}

/// The session as Hermes lists it: a turn under way or not.
fn runs(gateway: &Fake, running: bool) {
    {
        let mut world = gateway.world();
        let stream = world.stream(SESSION);
        stream.epoch = "epoch-1".into();
        stream.running = running;
    }
    gateway.say("sessions.changed");
}

#[test]
fn what_the_gateway_lists_reaches_core_and_goes_when_it_does() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let id = format!("hermes:{PROFILE}:{SESSION}");
    runs(&gateway, true);
    lab.follow();
    let running = vec![(id.clone(), LiveState::Running)];
    until("core is told the session runs", || told(&lab) == running);
    // Every poll that counts renews the reading.
    let first = read_ms(&lab);
    until("a later poll renews it", || read_ms(&lab) > first);

    runs(&gateway, false);
    let idle = vec![(id.clone(), LiveState::Idle)];
    until("core is told the turn is over", || told(&lab) == idle);

    // The gateway goes away: nothing it said is held any more.
    runs(&gateway, true);
    until("core is told it runs again", || told(&lab) == running);
    gateway.kill();
    until("core forgets the gateway's states", || {
        told(&lab).is_empty()
    });
}

#[test]
fn an_observer_that_stops_clears_what_it_told() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    runs(&gateway, true);
    lab.follow();
    until("core is told the session runs", || told(&lab).len() == 1);
    lab.observer = None;
    assert!(told(&lab).is_empty(), "stopping clears at once");
    assert!(lab.requests.native_states().is_empty());
}
