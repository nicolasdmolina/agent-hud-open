//! Never two readers of one home: what the far side holds by itself, and
//! what the caller's slot adds, through a killed client, a dropped link,
//! a changed address, a late start, a relaunch and a rebuilt machine.
//! Every count is the far side's own (`world::Watch`).
use super::super::{Failure, Hold, Reader, Slots};
use super::world::{
    clients, kill, machine_of, most, naming, open, raw, readers, sessions, settings, Watch, World,
    PYTHON,
};
use crate::ssh_link::unix::tests::lab::until;
use crate::ssh_link::{Cut, End, Folder, Link, Refused, Settings as LinkSettings, Target};
use agent_hud_agents::hermes::far_history::{Reply, Request};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

/// Given to the test program when it is started as a HUD to be killed.
const AS_A_HUD: &str = "AGENT_HUD_LAB_FAR_HISTORY_HUD";

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn two_readers_started_together() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let link = world.link();
    let request = sessions(&["endless"]);
    // Each holds the database until its deadline, which is long enough
    // for the two to overlap however slowly either starts.
    for round in 0..2 {
        let watch = Watch::start(&world);
        let together = Barrier::new(2);
        let run = || {
            together.wait();
            raw(&link, &world.home, Duration::from_secs(5), &request)
        };
        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(run);
            let second = scope.spawn(run);
            (first.join().unwrap(), second.join().unwrap())
        });
        let samples = watch.finish();
        let said = |ran: &crate::ssh_link::Ran| String::from_utf8_lossy(&ran.stdout).into_owned();
        println!(
            "  round {round}: {:?} {:?} / {:?} {:?}",
            first.end,
            said(&first),
            second.end,
            said(&second)
        );
        // One started, and held the database until its deadline. The
        // other found it and opened nothing.
        let mut ends = [(first.end, said(&first)), (second.end, said(&second))];
        ends.sort_by_key(|(_, said)| said.clone());
        assert_eq!(ends[0], (End::Ended(0), "[\"no\",\"busy\"]\n".to_owned()));
        assert_eq!(ends[1].0, End::Ended(3));
        assert!(ends[1].1.starts_with("[\"started\","), "{}", ends[1].1);
        let mut opened: Vec<u32> = samples.iter().flat_map(|look| look.open.clone()).collect();
        opened.sort_unstable();
        opened.dedup();
        assert_eq!(opened.len(), 1, "exactly one opened the database");
        assert_eq!(most(&samples).1, 1);
        assert!(until(Duration::from_secs(5), || readers(&world.mark()).is_empty()));
    }
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn never_two_readers() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let (reader, slots) = (world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let link = world.link();
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let request = sessions(&["endless"]);
    let read = |link: &Link| reader.read(link, &slot, &world.home, &request);
    let watch = Watch::start(&world);
    thread::scope(|scope| {
        // A read, and the local `ssh` client killed in the middle of it.
        let first = scope.spawn(|| read(&link));
        let database = world.database().display().to_string();
        assert!(until(Duration::from_secs(20), || {
            matches!(slot.hold(), Hold::Until(_))
                && !open(&readers(&world.mark()), &database).is_empty()
        }));
        let Hold::Until(held_until) = slot.hold() else {
            unreachable!()
        };
        let far_reader = readers(&world.mark());
        for client in clients(&world.mark()) {
            kill(client);
        }
        assert_eq!(
            first.join().unwrap(),
            Err(Failure::CutOff(Cut::ChannelClosed))
        );
        assert_eq!(slot.hold(), Hold::Until(held_until));
        // A new read is asked for at once.
        let second = scope.spawn(|| read(&link));
        thread::sleep(Duration::from_millis(200));
        // Then the link is dropped and opened again, and a read asked.
        link.close();
        let again = Arc::new(world.link());
        let (read, held) = (&read, Arc::clone(&again));
        let third = scope.spawn(move || read(&held));
        // Then the entry points at another address for the same machine.
        let other = Target::new("other", None, None, None).unwrap();
        let moved = Arc::new(Link::open(&world.folder, &other, &world.lab.settings()).unwrap());
        let held = Arc::clone(&moved);
        let fourth = scope.spawn(move || read(&held));
        // No other reader has started: the one that was cut off is
        // there alone, or has reached its deadline already.
        let now = readers(&world.mark());
        assert!(
            now.iter().all(|pid| far_reader.contains(pid)),
            "{now:?} beside {far_reader:?}"
        );

        // The read on the closed link is not run. The other two run one
        // after the other, each until its deadline.
        assert_eq!(
            second.join().unwrap(),
            Err(Failure::Link(Refused::Closed.into()))
        );
        assert_eq!(third.join().unwrap(), Err(Failure::Exit(3)));
        assert_eq!(fourth.join().unwrap(), Err(Failure::Exit(3)));
        let samples = watch.finish();
        // Through all of it, the far side never had two reader processes.
        assert_eq!(most(&samples), (1, 1));
        let mut seen: Vec<u32> = samples
            .iter()
            .flat_map(|look| look.readers.clone())
            .collect();
        seen.dedup();
        assert_eq!(
            seen.len(),
            3,
            "three readers ran, one after another: {seen:?}"
        );
        // And none was started before the cut-off one was sure to be gone.
        let next = samples
            .iter()
            .find(|look| look.readers.iter().any(|pid| !far_reader.contains(pid)))
            .unwrap();
        assert!(
            next.at >= held_until,
            "a reader started {:?} early",
            held_until - next.at
        );
        again.close();
        moved.close();
    });
}

/// The far side starts a reader five seconds late: later than its
/// deadline (1 s) and the margin (2 s) together, long after the caller
/// gave up on it. `shown` says whether the far process list already shows
/// the late starter while it waits, as it does once its shell runs, or
/// not yet, as when the far machine has not begun it at all.
fn a_late_start(shown: bool) {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let mut short = settings();
    short.deadline = Duration::from_secs(1);
    let mut impatient = short.clone();
    impatient.local_wait = Duration::from_millis(1_200);
    let (reader, hasty, slots) = (world.reader(short), world.reader(impatient), Slots::new());
    let slot = slots.of(&world.machine());
    let link = world.link();
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let request = sessions(&["endless"]);
    // While the flag is there, a reader's start is held back.
    let at = |name: &str| world.lab.at(name).display().to_string();
    fs::write(at("late"), "").unwrap();
    if shown {
        world.stand_in(&format!(
            "if [ -e '{}' ]; then sleep 5; fi\nexec {PYTHON} \"$@\"",
            at("late")
        ));
    } else {
        // The wait is in a process that does not carry the reader's
        // arguments, so nothing on the far side shows a reader yet.
        fs::write(
            at("late.sh"),
            format!(
                "sleep 5\nset --\nwhile IFS= read -r one; do set -- \"$@\" \"$one\"; done < '{}'\nexec {PYTHON} \"$@\"\n",
                at("late-arguments")
            ),
        )
        .unwrap();
        world.stand_in(&format!(
            "if [ -e '{}' ]; then printf '%s\\n' \"$@\" > '{}'; exec /bin/sh '{}'; fi\nexec {PYTHON} \"$@\"",
            at("late"),
            at("late-arguments"),
            at("late.sh")
        ));
    }
    let watch = Watch::start(&world);
    let began = Instant::now();
    // The caller cuts off before `started`.
    assert_eq!(
        hasty.read(&link, &slot, &world.home, &request),
        Err(Failure::CutOff(Cut::TimeLimit))
    );
    assert_eq!(slot.hold(), Hold::StartUnknown);
    let late = if shown {
        readers(&world.mark())
    } else {
        naming(&at("late.sh"))
    };
    assert_eq!(late.len(), 1, "the late starter waits on the far side");
    assert_eq!(readers(&world.mark()).len(), usize::from(shown));
    fs::remove_file(at("late")).unwrap();
    // And asks for a new read at once, and keeps asking.
    let mut answers = Vec::new();
    while began.elapsed() < Duration::from_secs(11) {
        let answer = reader.read(&link, &slot, &world.home, &request);
        answers.push((began.elapsed().as_millis(), answer));
        thread::sleep(Duration::from_millis(100));
    }
    assert!(until(Duration::from_secs(8), || readers(&world.mark()).is_empty()));
    let samples = watch.finish();
    for (at, answer) in &answers {
        println!("  {at} ms: {answer:?}");
    }
    // The first read after the cut-off went through the probe.
    let log = world.logged();
    let cut = log
        .iter()
        .position(|line| line.ends_with("started not seen"))
        .unwrap();
    assert!(log[cut + 1].contains("probes first"), "{log:?}");
    if shown {
        // A late starter that is seen keeps every new reader out.
        assert_eq!(answers[0].1, Err(Failure::Busy));
    } else {
        // One that is not there yet does not: the probe started, so the
        // read ran, long before the late one did.
        assert_eq!(answers[0].1, Err(Failure::Exit(3)));
    }
    assert!(answers
        .iter()
        .all(|(_, answer)| matches!(answer, Err(Failure::Exit(3) | Failure::Busy))));
    assert!(answers
        .iter()
        .any(|(_, answer)| *answer == Err(Failure::Exit(3))));
    // Never two readers with the database open.
    assert_eq!(most(&samples).1, 1);
    // The late starter is gone, and it either found a reader and opened
    // nothing, or had the database to itself.
    let alone = samples.iter().filter(|look| look.open == late).count();
    let beside = samples
        .iter()
        .filter(|look| look.open.contains(&late[0]) && look.open.len() > 1)
        .count();
    let busy = answers
        .iter()
        .filter(|(_, answer)| *answer == Err(Failure::Busy))
        .count();
    println!(
        "  the late starter {late:?}: alone with the database in {alone} looks, beside another in {beside}; the caller was told busy {busy} times"
    );
    assert_eq!(beside, 0);
    assert!(!late
        .iter()
        .any(|pid| crate::ssh_link::unix::tests::lab::runs(*pid)));
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn delayed_remote_startup_does_not_overlap() {
    a_late_start(false);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_late_starter_the_far_side_shows_keeps_new_readers_out() {
    a_late_start(true);
}

/// Not a test of its own: the HUD that `hud_relaunch_does_not_overlap_a_reader`
/// starts and kills. Run without its variable it does nothing.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_hud_that_reads_until_it_is_killed() {
    let Ok(given) = std::env::var(AS_A_HUD) else {
        return;
    };
    let mut given = given.lines();
    let mut next = || given.next().unwrap().to_owned();
    let (config, folder, home) = (next(), next(), next());
    let link_settings = LinkSettings {
        config: Some(PathBuf::from(config)),
        connect_timeout: Duration::from_secs(10),
        alive_interval: Duration::from_secs(1),
        alive_count: 2,
        ..LinkSettings::system()
    };
    let folder = Folder::open(&PathBuf::from(folder)).unwrap();
    let link = Link::open(
        &folder,
        &Target::new("lab", None, None, None).unwrap(),
        &link_settings,
    )
    .unwrap();
    let mut long = settings();
    // Long enough to be running still when the HUD is back, however
    // busy this machine is.
    long.deadline = Duration::from_secs(15);
    let reader = Reader::new(long, |line| println!("  hud log: {line}"));
    let slots = Slots::new();
    loop {
        let _ = reader.read(
            &link,
            &slots.of(&machine_of(&home)),
            &home,
            &sessions(&["endless"]),
        );
    }
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn hud_relaunch_does_not_overlap_a_reader() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let database = world.database().display().to_string();
    let given = [
        world.lab.at("config").display().to_string(),
        world.folder.path().display().to_string(),
        world.home.clone(),
    ];
    let mut hud = Command::new(std::env::current_exe().unwrap())
        .args([
            "clients::hermes::far_history::tests::exclusion::a_hud_that_reads_until_it_is_killed",
            "--exact",
            "--ignored",
            "--nocapture",
        ])
        .env(AS_A_HUD, given.join("\n"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let watch = Watch::start(&world);
    // The HUD is killed in the middle of a query.
    assert!(until(Duration::from_secs(30), || !open(
        &readers(&world.mark()),
        &database
    )
    .is_empty()));
    let old = readers(&world.mark());
    kill(hud.id());
    hud.wait().unwrap();
    // Relaunched: the control folder is swept, as at every launch, a new
    // link opens, nothing is known of any reader, and a read is asked for
    // at once.
    let swept = world.folder.sweep(&world.lab.settings());
    println!("  relaunch swept {swept:?}");
    let link = world.link();
    let (reader, slots) = (world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    assert_eq!(slot.hold(), Hold::StartUnknown);
    let request = sessions(&["endless"]);
    let first = reader.read(&link, &slot, &world.home, &request);
    // It went through the probe, which found the old reader alive.
    assert!(
        world.logged()[0].contains("probes first"),
        "{:?}",
        world.logged()
    );
    assert_eq!(first, Err(Failure::Busy));
    assert_eq!(readers(&world.mark()), old, "the old reader is still there");
    assert_eq!(slot.hold(), Hold::StartUnknown);
    // Busy turns into started on a later try.
    let mut tries = 1;
    let began = Instant::now();
    loop {
        tries += 1;
        match reader.read(&link, &slot, &world.home, &request) {
            Err(Failure::Busy) => thread::sleep(Duration::from_millis(200)),
            other => {
                assert_eq!(other, Err(Failure::Exit(3)));
                break;
            }
        }
        assert!(began.elapsed() < Duration::from_secs(90));
    }
    let samples = watch.finish();
    println!(
        "  the read went through on try {tries}, {:?} after the relaunch",
        began.elapsed()
    );
    assert!(tries > 2);
    assert_eq!(most(&samples).1, 1);
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn retire_and_readd_keeps_reader_exclusion() {
    let Some(world) = World::start() else { return };
    world.make_every_read_endless();
    let database = world.database().display().to_string();
    // The first reader lives long enough to be running still when the
    // machine is back, however busy this machine is. The reads after
    // that are short ones.
    let mut long = settings();
    long.deadline = Duration::from_secs(12);
    let (reader, short, slots) = (world.reader(long), world.reader(settings()), Slots::new());
    let slot = slots.of(&world.machine());
    let link = world.link();
    assert_eq!(
        reader.read(&link, &slot, &world.home, &Request::Probe),
        Ok(Reply::Probed)
    );
    let request = sessions(&["endless"]);
    let watch = Watch::start(&world);
    thread::scope(|scope| {
        let first = scope.spawn(|| reader.read(&link, &slot, &world.home, &request));
        assert!(until(Duration::from_secs(20), || {
            matches!(slot.hold(), Hold::Until(_))
                && !open(&readers(&world.mark()), &database).is_empty()
        }));
        let old = readers(&world.mark());
        // The machine's only entry is removed in the middle of the query:
        // the machine is retired and its link closed.
        slots.retired(&world.machine());
        link.close();
        assert!(first.join().unwrap().is_err());
        // And added again at once. It has the slot it had, which now
        // knows nothing of the reader.
        let same = slots.of(&world.machine());
        assert!(Arc::ptr_eq(&slot, &same));
        assert_eq!(same.hold(), Hold::StartUnknown);
        let again = world.link();
        let before = world.logged().len();
        let read = short.read(&again, &same, &world.home, &request);
        assert!(world.logged()[before].contains("probes first"));
        assert_eq!(read, Err(Failure::Busy));
        assert_eq!(readers(&world.mark()), old, "the old reader is still there");
        let began = Instant::now();
        loop {
            match short.read(&again, &same, &world.home, &request) {
                Err(Failure::Busy) => thread::sleep(Duration::from_millis(200)),
                other => {
                    assert_eq!(other, Err(Failure::Exit(3)));
                    break;
                }
            }
            assert!(began.elapsed() < Duration::from_secs(90));
        }
        again.close();
    });
    let samples = watch.finish();
    assert_eq!(most(&samples).1, 1);
}

/// An older reader can have the higher process id, once the system's
/// ids have gone round. The newer one, with the lower id, waits three
/// seconds for it to leave, as it would for one that started beside it,
/// and then answers "busy". With a deadline shorter than that wait, its
/// watchdog ended it first, with the deadline's status and no answer:
/// the time a reader had to start in was its deadline. Found by a run of
/// these tests in which the ids went round.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_reader_that_waits_for_a_higher_process_id_answers_busy_before_its_deadline() {
    let Some(world) = World::start() else { return };
    world.add_session(&world.db(), "s1", 1);
    let link = world.link();
    let mark = world.mark();
    // The reader's process is there at once and looks 0.7 s later, by
    // when another process that carries its mark has started: a later
    // one, so one with a higher id.
    world.stand_in(&format!("sleep 0.7\nexec {PYTHON} \"$@\""));
    for deadline in [2, 1] {
        let began = Instant::now();
        let (ran, lower, higher) = thread::scope(|scope| {
            let read = scope.spawn(|| {
                raw(
                    &link,
                    &world.home,
                    Duration::from_secs(deadline),
                    &sessions(&["s1"]),
                )
            });
            assert!(until(Duration::from_secs(5), || !readers(&mark).is_empty()));
            let lower = readers(&mark)[0];
            let mut other = Command::new(PYTHON)
                .args(["-I", "-c", "import time; time.sleep(20)", &mark])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let higher = other.id();
            let ran = read.join().unwrap();
            other.kill().unwrap();
            other.wait().unwrap();
            (ran, lower, higher)
        });
        let took = began.elapsed();
        println!(
            "  deadline {deadline} s: the reader {lower} beside {higher} => {:?}, {:?} after {took:?}",
            ran.end,
            String::from_utf8_lossy(&ran.stdout)
        );
        if higher < lower {
            // The ids went round between the two: the other one is the
            // lower, and the reader says so at once.
            println!("  the ids went round here: nothing to wait for");
        }
        assert_eq!(ran.end, End::Ended(0));
        assert_eq!(ran.stdout, b"[\"no\",\"busy\"]\n");
        assert!(until(Duration::from_secs(5), || readers(&mark).is_empty()));
    }
    // With nobody beside it, the same reader reads.
    let ran = raw(
        &link,
        &world.home,
        Duration::from_secs(2),
        &sessions(&["s1"]),
    );
    assert_eq!(ran.end, End::Ended(0));
    assert!(String::from_utf8_lossy(&ran.stdout).starts_with("[\"started\","));
    link.close();
}
