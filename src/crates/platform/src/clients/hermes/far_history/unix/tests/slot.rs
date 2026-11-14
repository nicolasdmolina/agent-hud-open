//! The caller's slot: what holds it, and what frees it.
use super::super::{Failure, Hold, Slots};
use super::world::{
    clients, kill, machine_of, most, readers, sessions, settings, Watch, World, PYTHON,
};
use crate::ssh_link::unix::tests::lab::until;
use crate::ssh_link::Cut;
use agent_hud_agents::hermes::far_history::{Reply, Request};
use std::{
    sync::Barrier,
    thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_cut_off_reader_holds_the_slot() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    assert_eq!(slot.hold(), Hold::Free);
    let request = sessions(&["endless"]);
    let watch = Watch::start(&world);
    let (held_until, old) = thread::scope(|scope| {
        let first = scope.spawn(|| reader.read(&link, &slot, &world.home, &request));
        // Cut off after `started`: the local client is killed once the
        // slot has seen the line.
        let seen = Instant::now();
        assert!(until(Duration::from_secs(20), || matches!(
            slot.hold(),
            Hold::Until(_)
        )));
        let Hold::Until(held_until) = slot.hold() else {
            unreachable!()
        };
        // The slot holds for the deadline and the margin from when it saw
        // `started`.
        let held = held_until - seen;
        assert!(
            held <= Duration::from_secs(4) + Duration::from_secs(20)
                && held > Duration::from_millis(3_900),
            "{held:?}"
        );
        let old = readers(&world.mark());
        for client in clients(&world.mark()) {
            kill(client);
        }
        assert_eq!(
            first.join().unwrap(),
            Err(Failure::CutOff(Cut::ChannelClosed))
        );
        (held_until, old)
    });
    assert_eq!(slot.hold(), Hold::Until(held_until));
    // The next read is asked for at once and starts only after that.
    let asked = Instant::now();
    assert!(asked < held_until);
    assert_eq!(
        reader.read(&link, &slot, &world.home, &request),
        Err(Failure::Exit(3))
    );
    let samples = watch.finish();
    let next = samples
        .iter()
        .find(|look| look.readers.iter().any(|pid| !old.contains(pid)))
        .unwrap();
    println!(
        "  asked {:?} before the slot was free; the next reader appeared {:?} after it",
        held_until - asked,
        next.at - held_until
    );
    assert!(next.at >= held_until);
    assert_eq!(most(&samples), (1, 1));
    // The log says why it waited.
    let log = world.logged();
    let why = log
        .iter()
        .find(|line| line.contains("waits"))
        .expect("a line for the wait");
    assert!(
        why.ends_with("ms: a reader cut off after it started may run until its deadline"),
        "{why}"
    );
    assert_eq!(slot.hold(), Hold::Free);
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn cutoff_before_started_is_never_freed_by_time() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 2);
    let mut short = settings();
    short.deadline = Duration::from_millis(500);
    short.local_wait = Duration::from_millis(1_500);
    let (link, reader, slots) = (world.link(), world.reader(short), Slots::new());
    let slot = slots.of(&world.machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    assert_eq!(slot.hold(), Hold::Free);
    // A far side that gives no reply at all before the caller gives up.
    world.stand_in(&format!("sleep 4\nexec {PYTHON} \"$@\""));
    let request = sessions(&["s1"]);
    assert_eq!(
        reader.read(&link, &slot, &world.home, &request),
        Err(Failure::CutOff(Cut::TimeLimit))
    );
    assert!(world.logged().pop().unwrap().ends_with("started not seen"));
    assert_eq!(slot.hold(), Hold::StartUnknown);
    world.no_stand_in();
    // Ten deadlines later, and the margin, the slot is held still.
    let waited = Instant::now();
    while waited.elapsed() < Duration::from_millis(10 * 500 + 2_000) {
        assert_eq!(slot.hold(), Hold::StartUnknown);
        thread::sleep(Duration::from_millis(100));
    }
    assert!(until(Duration::from_secs(10), || readers(&world.mark())
        .is_empty()));
    assert_eq!(slot.hold(), Hold::StartUnknown);
    // Only a probe frees it: the next read makes one, which starts.
    let before = world.logged().len();
    let read = reader.read(&link, &slot, &world.home, &request);
    assert!(
        matches!(&read, Ok(Reply::Sessions(batch)) if batch.sessions.len() == 1),
        "{read:?}"
    );
    let log = world.logged();
    assert!(log[before].contains("probes first"), "{log:?}");
    assert!(log[before + 1].starts_with("history probe ok"), "{log:?}");
    assert_eq!(slot.hold(), Hold::Free);
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn two_requests_at_once() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 2);
    let (link, reader, slots) = (world.link(), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    // A slow far side: each reader takes a second to start.
    world.stand_in(&format!("sleep 1\nexec {PYTHON} \"$@\""));
    let request = sessions(&["s1"]);
    let watch = Watch::start(&world);
    let together = Barrier::new(2);
    let run = || {
        together.wait();
        let answer = reader.read(&link, &slot, &world.home, &request);
        (answer, Instant::now())
    };
    let ((first, first_end), (second, second_end)) = thread::scope(|scope| {
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        (first.join().unwrap(), second.join().unwrap())
    });
    let samples = watch.finish();
    assert!(matches!(first, Ok(Reply::Sessions(_))) && matches!(second, Ok(Reply::Sessions(_))));
    // Two far readers ran, and the second appeared only after the first
    // had gone and its answer had ended here.
    assert_eq!(most(&samples).0, 1);
    let mut pids: Vec<u32> = samples
        .iter()
        .flat_map(|look| look.readers.clone())
        .collect();
    pids.dedup();
    assert_eq!(pids.len(), 2, "{pids:?}");
    let last_of_first = samples
        .iter()
        .rfind(|look| look.readers == [pids[0]])
        .unwrap()
        .at;
    let first_of_second = samples
        .iter()
        .find(|look| look.readers == [pids[1]])
        .unwrap()
        .at;
    let earlier_end = first_end.min(second_end);
    println!(
        "  the second reader appeared {:?} after the first one's last look, {:?} after the first answer ended",
        first_of_second - last_of_first,
        first_of_second.saturating_duration_since(earlier_end)
    );
    assert!(first_of_second > last_of_first);
    assert!(first_of_second + Duration::from_millis(40) >= earlier_end);
    link.close();
}

/// The slot by itself, with no reader run.
#[test]
fn a_slot_starts_held_and_a_retired_machine_keeps_its_slot() {
    let (slots, machine) = (Slots::new(), machine_of("/far/.hermes"));
    let slot = slots.of(&machine);
    // Nothing is known at launch.
    assert_eq!(slot.hold(), Hold::StartUnknown);
    let (round, before) = slot.begin();
    assert_eq!(before, Hold::StartUnknown);
    slot.ended(round);
    assert_eq!(slot.hold(), Hold::Free);

    // A reader started, seen to start, and never seen to end.
    let (round, before) = slot.begin();
    assert_eq!((before, slot.hold()), (Hold::Free, Hold::StartUnknown));
    let until = Instant::now() + Duration::from_secs(60);
    slot.started(round, until);
    assert_eq!(slot.hold(), Hold::Until(until));
    // The time has not passed, so it is not freed.
    slot.lapsed(until);
    assert_eq!(slot.hold(), Hold::Until(until));

    // The machine is retired while it is held: nothing is known any
    // more, and what the old reader's caller learns later changes nothing.
    slots.retired(&machine);
    assert_eq!(slot.hold(), Hold::StartUnknown);
    slot.ended(round);
    slot.started(round, until);
    assert_eq!(slot.hold(), Hold::StartUnknown);
    assert!(std::sync::Arc::ptr_eq(&slot, &slots.of(&machine)));

    // A slot that was free when its machine was retired stays free.
    let (round, _) = slot.begin();
    slot.ended(round);
    slots.retired(&machine);
    assert_eq!(slot.hold(), Hold::Free);
    // A read that the link refused before it ran `ssh` leaves the slot
    // as it was, and one told "busy" knows nothing of the other reader.
    let (round, before) = slot.begin();
    slot.restore(round, before);
    assert_eq!(slot.hold(), Hold::Free);
    let (round, _) = slot.begin();
    slot.busy(round);
    assert_eq!(slot.hold(), Hold::StartUnknown);
    // A time that has passed frees it.
    let (round, _) = slot.begin();
    let past = Instant::now();
    slot.started(round, past);
    slot.lapsed(past);
    assert_eq!(slot.hold(), Hold::Free);
}
