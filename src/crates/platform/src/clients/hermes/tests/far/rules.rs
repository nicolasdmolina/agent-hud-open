//! The rules the route over SSH keeps from the tickets before it: a stop
//! ends what was begun, a lost link is never word that a gateway is
//! gone, the far side's own checks decide whether a machine is covered,
//! the key is made from the root home as the far side resolved it, and
//! turning the channel off leaves no master behind.
use super::{
    super::{
        super::{
            control::Silent,
            route::{ssh::Ssh, Route, Stop},
        },
        fake::{fixture, Fake},
        marked,
    },
    far_lab, lab, until, Standing, INSTALL,
};
use agent_hud_agents::hermes::gateway::{
    identify, identify_line, machine_key, Identified, Instance,
};
use std::{
    os::unix::fs::PermissionsExt, path::Path, process::Command, sync::atomic::Ordering,
    time::Duration,
};

/// Someone who asks and has stopped already.
struct Stopped;
impl Stop for Stopped {
    fn wait(&self, _time: Duration) -> bool {
        true
    }
}

/// The ports the master of the route's open link listens on.
fn listening(route: &Ssh) -> Vec<String> {
    let (link, _) = route.link().expect("a link");
    let pid = link.master_pid().expect("its master").to_string();
    let listed = Command::new("lsof")
        .args(["-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-P", "-n", "-Fn"])
        .output()
        .expect("lsof");
    let said = String::from_utf8_lossy(&listed.stdout).into_owned();
    said.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::to_owned)
        .collect()
}

/// The instance the gateway in the far root answers as, read as the
/// model reads it.
fn instance(route: &Ssh) -> Instance {
    let home = route.homes().expect("the far homes").remove(0);
    let line = format!("{}\n", identify_line());
    let answer = route
        .exchange(&home, line.as_bytes(), Duration::from_secs(5))
        .expect("an answer");
    let answer = serde_json::from_slice(&answer[..answer.len() - 1]).unwrap();
    match identify(&answer, &route.named(&home)) {
        Identified::Ready(instance) => instance,
        other => panic!("not ready: {other:?}"),
    }
}

/// Someone who asks and goes on wanting it.
struct Wanted;
impl Stop for Wanted {
    fn wait(&self, time: Duration) -> bool {
        std::thread::sleep(time);
        false
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_forward_asked_for_by_someone_who_stopped_is_cancelled_and_not_kept() {
    let Some(far) = far_lab() else {
        return;
    };
    let _gateway = far.gateway("instance-1");
    // The route alone: nothing else makes a forward over it.
    let (folder, target) = (far.lab.folder(), far.lab.target());
    let route = Ssh::start(
        folder,
        target,
        far.lab.settings(),
        std::sync::Arc::new(|_| {}),
    )
    .expect("the route");
    until("the link is up", || route.link().is_some());
    let instance = instance(&route);
    assert_eq!(listening(&route), Vec::<String>::new());

    // Asked for by someone who stopped: refused, and whatever was begun
    // for it is ended.
    assert!(route.prepare(&instance, &Stopped).is_err());
    assert!(route.dial(&instance).is_err(), "nothing to dial through");
    std::thread::sleep(Duration::from_millis(800));
    assert!(route.dial(&instance).is_err(), "nothing was kept for it");
    assert_eq!(listening(&route), Vec::<String>::new());
    assert_eq!(route.forwards_made(), 0, "none was kept as made");

    // Asked for by someone who still wants it: made, once.
    route.prepare(&instance, &Wanted).expect("a forward");
    assert_eq!(route.forwards_made(), 1);
    let kept = listening(&route);
    assert_eq!(kept.len(), 1, "{kept:?}");
    route.prepare(&instance, &Wanted).expect("the same forward");
    assert_eq!(route.forwards_made(), 1);
    assert_eq!(listening(&route), kept);

    // Another instance at that home replaces it: the old forward is
    // cancelled before the new one is in place.
    let mut other = instance.clone();
    other.id = "instance-2".into();
    route.prepare(&other, &Wanted).expect("a forward");
    assert_eq!(route.forwards_made(), 2);
    let now = listening(&route);
    assert_eq!(now.len(), 1, "one forward for the gateway: {now:?}");
    assert_ne!(now, kept, "the old instance's forward was cancelled");
    assert!(route.dial(&instance).is_err(), "the old one leads nowhere");
    drop(route.dial(&other).expect("a stream through the forward"));

    // An instance that is followed no more has its forward cancelled,
    // whatever home it was asked through.
    route.release(&other.id);
    until("the forward is cancelled", || listening(&route).is_empty());
    assert!(route.dial(&other).is_err());
    route.stop();
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_lost_link_is_no_word_that_a_gateway_is_gone_and_a_home_list_is_never_whole() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    far.follow();
    far.followed();
    gateway.play("question-answered-here", Some("clarify.request"));
    far.card();
    let route = &far.inner().route;
    let told = route.inventory().expect("the far homes");
    assert!(!told.whole, "a far list never proves a home gone");
    assert_eq!(told.homes.len(), 1);

    // The link is held down: every asking fails, and none of them says
    // that nobody listens.
    let config = far.lab.config_text();
    let from = format!("Port {}", far.lab.port);
    let nowhere = config.replace(&from, &format!("Port {}", lab::closed_port()));
    std::fs::write(far.lab.at("config"), nowhere).unwrap();
    let (link, _) = route.link().unwrap();
    let followers = far.inner().followers().unwrap();
    // Watched from before the link goes, without a pause: a follower
    // taken for gone even for a moment gives up what it claims.
    let watched: Vec<_> = followers.set().clone();
    let over = std::sync::atomic::AtomicBool::new(false);
    let ever_gone = std::thread::scope(|scope| {
        let watch = scope.spawn(|| {
            let mut ever = false;
            while !over.load(Ordering::Acquire) {
                ever |= watched.iter().any(|one| one.gone.load(Ordering::Acquire));
                std::thread::sleep(Duration::from_micros(200));
            }
            ever
        });
        // SAFETY: a signal to the master this test's observer started.
        unsafe { libc::kill(link.master_pid().unwrap() as libc::pid_t, libc::SIGKILL) };
        until("the card is marked", || far.cards().iter().all(marked));
        until("the link is known to be down", || route.link().is_none());
        assert!(route.inventory().is_err(), "the homes cannot be told");
        std::thread::sleep(Duration::from_secs(2));
        over.store(true, Ordering::Release);
        watch.join().unwrap()
    });
    assert!(
        !ever_gone,
        "a follower was taken for gone when its link was lost"
    );
    assert_eq!(far.cards().len(), 1);

    // Stopped for real, over a link that stands: now it is gone, and its
    // card still stays, marked.
    std::fs::write(far.lab.at("config"), config).unwrap();
    until("the link is back", || route.link().is_some());
    until("the mark clears", || !far.cards().iter().any(marked));
    gateway.kill();
    until("the follower finds its gateway gone", || {
        let set = followers.set();
        set.iter()
            .all(|follower| follower.gone.load(Ordering::Acquire))
    });
    until("the card is marked", || far.cards().iter().all(marked));
    assert_eq!(far.cards().len(), 1);
}

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_home_that_fails_a_check_uncovers_the_machine_before_and_after_it_is_followed() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    gateway.play("question-answered-here", Some("clarify.request"));
    // Others may write the far home: nothing is asked of its socket.
    chmod(&far.root, 0o777);
    far.follow();
    let open = Standing::NotCovered("home-open-to-others".into());
    until("not covered, with the check's own reason", || {
        far.standing() == open
    });
    assert!(far.cards().is_empty());
    assert!(gateway.world().granted.is_empty(), "no ticket was asked");
    assert_eq!(gateway.world().sockets, 0);

    // Closed again: followed, and its question shows.
    chmod(&far.root, 0o700);
    far.followed();
    far.card();

    // Opened up while it is followed: what was told is withdrawn.
    chmod(&far.root, 0o777);
    gateway.cut();
    until("not covered again", || far.standing() == open);
    until("its card is withdrawn", || far.cards().is_empty());
    let waiting = far.requests.native_waiting_sessions();
    assert!(waiting.values().all(|sessions| sessions.is_empty()));
    chmod(&far.root, 0o700);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn the_key_is_made_from_the_far_root_as_the_far_side_resolved_it() {
    let Some(mut far) = far_lab() else {
        return;
    };
    // The far root is a link: `~/.hermes` leads to a folder elsewhere.
    let place = far.root.parent().unwrap().to_owned();
    let (one, other) = (place.join("hermes-one"), place.join("hermes-other"));
    std::fs::remove_dir(&far.root).unwrap();
    for home in [&one, &other] {
        crate::os::private_directory(home).unwrap();
    }
    std::os::unix::fs::symlink(&one, &far.root).unwrap();
    let gateway = Fake::start(&one, "instance-one");
    gateway.world().install = Some(INSTALL.into());
    gateway.play("question-answered-here", Some("clarify.request"));
    far.follow();
    let key = far.followed();
    let resolved = machine_key(Some(INSTALL), one.to_str().unwrap()).unwrap();
    assert_eq!(key, resolved, "made from where the link leads");
    assert_ne!(
        key,
        machine_key(Some(INSTALL), far.root.to_str().unwrap()).unwrap()
    );
    let card = far.card();
    assert!(card["sessionId"].as_str().unwrap().contains(key.as_str()));

    // The same install in another root home is another machine: the
    // first one's card is withdrawn, and nothing is told under its key.
    let moved = Fake::start(&other, "instance-other");
    moved.world().install = Some(INSTALL.into());
    std::fs::remove_file(&far.root).unwrap();
    std::os::unix::fs::symlink(&other, &far.root).unwrap();
    let next = machine_key(Some(INSTALL), other.to_str().unwrap()).unwrap();
    until("followed as the other machine", || {
        far.standing() == Standing::Connected(next.clone())
    });
    assert!(far.log().contains("not covered why=root_home_changed"));
    until("the first machine's card is withdrawn", || {
        far.cards().is_empty()
    });
    moved.play("question-answered-here", Some("clarify.request"));
    let card = far.card();
    assert!(card["sessionId"].as_str().unwrap().contains(next.as_str()));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_gateway_without_the_live_connection_is_reported_and_not_followed() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    gateway.play("question-answered-here", Some("clarify.request"));
    // A build whose control socket knows no ticket.
    gateway.world().grants = false;
    far.follow();
    let none = Standing::NotCovered("no_live_connection".into());
    until("not covered: no live connection", || far.standing() == none);
    let lacking = |why: &'static str| vec![("default".to_owned(), why)];
    assert_eq!(far.observer().not_followed(), lacking("unknown_verb"));
    assert!(far
        .log()
        .contains("gateway not followed profile=default why=unknown_verb"));
    assert!(far.cards().is_empty());
    assert_eq!(gateway.world().sockets, 0);

    // An older build still, by what it says of itself.
    let old = fixture("gateway-self-description-old-build.SYNTHETIC");
    let mut told = old["cases"][0]["identify_result"].clone();
    told["hermes_home"] = far.root.to_string_lossy().into_owned().into();
    let identify = serde_json::json!({"ok": true, "protocol": 1, "id": 1, "result": told});
    gateway.world().identify = Some(identify);
    until("reported by its own reason", || {
        far.observer().not_followed() == lacking("runtime_protocol")
    });
    assert_eq!(far.standing(), none);
    assert!(far.cards().is_empty());
    assert!(!far.log().contains("followed machine="));
    assert!(gateway.world().calls("profiles.list").is_empty());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn turning_the_channel_off_withdraws_the_cards_and_leaves_no_master() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    far.follow();
    far.followed();
    gateway.play("question-answered-here", Some("clarify.request"));
    far.card();
    let folder = far.lab.folder();
    assert_eq!(far.lab.link_processes(&folder).len(), 1, "the one master");
    let sockets = |folder: &crate::ssh_link::Folder| {
        let names = lab::entries(folder.path());
        let plain = |name: &&String| !name.ends_with(".pid") && !name.ends_with(".log");
        names.iter().filter(plain).cloned().collect::<Vec<_>>()
    };
    let control = folder.path().join(&sockets(&folder)[0]);

    far.observer.take();
    assert!(far.cards().is_empty(), "withdrawn");
    let waiting = far.requests.native_waiting_sessions();
    assert!(waiting.values().all(|sessions| sessions.is_empty()));
    // No master, no socket, and nothing answers on the control path.
    assert_eq!(far.lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(lab::entries(folder.path()), Vec::<String>::new());
    let check = Command::new("/usr/bin/ssh")
        .args(["-F", "none", "-o"])
        .arg(format!("ControlPath={}", control.display()))
        .args(["-O", "check", "--", "link.invalid"])
        .output()
        .expect("ssh");
    assert!(!check.status.success(), "a master still answers");
    assert_eq!(far.log().matches("stopped").count(), 2, "{}", far.log());

    // One that is still signing in when it is turned off is ended too.
    let slow = far.lab.config_text().replace(
        &format!("Port {}", far.lab.port),
        "Port 9\n  ProxyCommand /bin/sleep 600",
    );
    std::fs::write(far.lab.at("config"), slow).unwrap();
    far.follow();
    until("a master is signing in", || {
        far.lab.link_processes(&folder).len() == 1
    });
    assert_eq!(far.standing(), Standing::Connecting);
    far.observer.take();
    until("it is ended", || far.lab.link_processes(&folder).is_empty());
    until("and its files are gone", || {
        lab::entries(folder.path()).is_empty()
    });
}

/// A command given to a master that is gone never starts, so the route
/// hands such a link none: the asking fails as not heard, and the home is
/// not kept from being asked once the link is back. (A relay that was
/// under way when its link went may still run, and its home waits out
/// its deadline: ticket 29's rule.)
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_master_that_is_gone_is_handed_no_command_and_its_homes_are_asked_again_at_once() {
    let Some(far) = far_lab() else {
        return;
    };
    let _gateway = far.gateway("instance-1");
    // The route alone: nothing else asks over it.
    let (folder, target) = (far.lab.folder(), far.lab.target());
    let route = Ssh::start(
        folder,
        target,
        far.lab.settings(),
        std::sync::Arc::new(|_| {}),
    )
    .expect("the route");
    until("the link is up", || route.link().is_some());
    let home = route.homes().expect("the far homes").remove(0);
    let line = format!("{}\n", identify_line());
    let ask = || route.exchange(&home, line.as_bytes(), Duration::from_secs(5));
    assert!(ask().is_ok());

    let (link, first) = route.link().unwrap();
    // SAFETY: a signal to the master this test's route started.
    unsafe { libc::kill(link.master_pid().unwrap() as libc::pid_t, libc::SIGKILL) };
    until("the master is gone", || link.running().is_err());
    // Asked while the keeper may not have seen it yet: not heard, and
    // never nobody listening.
    assert_eq!(ask().err(), Some(Silent::TimedOut));
    assert!(route.inventory().is_err());

    // The next link answers at the first asking: no home waits.
    until("the link is back", || {
        route.link().is_some_and(|(_, number)| number > first)
    });
    let home = route.homes().expect("the far homes again").remove(0);
    let line = format!("{}\n", identify_line());
    let answer = route.exchange(&home, line.as_bytes(), Duration::from_secs(5));
    assert!(answer.is_ok(), "{:?}", answer.err());
    route.stop();
}
