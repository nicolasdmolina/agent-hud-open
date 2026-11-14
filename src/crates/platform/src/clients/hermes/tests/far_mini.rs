//! A real far machine, followed and only read. Run by hand, against a
//! host of the user's own `ssh` configuration:
//! `AGENT_HUD_FAR_HOST=<host> AGENT_HUD_FAR_RECEIPT=<folder> cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_mini -- --ignored --nocapture`.
//!
//! It is the one test that reads the user's `ssh` configuration, since
//! that is where the host is named. It writes none of those files: the
//! link refuses a host whose key is not already trusted, and its master
//! and control socket are its own, in a folder of this test.
//!
//! On the far machine it runs four commands that read (free disk, the
//! hashes of the profiles' `config.yaml`, whether the install has its id,
//! where `python3` is), then what the app runs: the homes command
//! and the relay. Over the gateways' WebSockets it sends only reads. No
//! visit begins (`tap.rs`), so no session is subscribed for a snapshot
//! or an answer, and nothing answers a card: the cards go to requests of
//! this test's own, which nobody answers.
use super::super::{tap, FarEntry, FarObserver, Pace, Standing};
use crate::ssh_link::{
    unix::tests::{lab, transcript},
    End, FarCommand, Folder, Link, Settings, Target,
};
use agent_hud_agents::hermes::gateway::Connection;
use agent_hud_core::requests::Requests;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

/// The least free disk the far machine must have, in KiB: 5 GB.
const LEAST_FREE_KB: u64 = 5 * 1024 * 1024;
/// How many polls of each gateway are timed.
const POLLS: usize = 100;
const ROOT: &str = "${HERMES_HOME:-$HOME/.hermes}";

/// What the test did and found, written to the receipt folder whether
/// it ends well or not.
struct Receipt {
    folder: PathBuf,
    lines: Vec<String>,
}

impl Receipt {
    fn say(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("{line}");
        self.lines.push(line);
    }
}

impl Drop for Receipt {
    fn drop(&mut self) {
        let mut all = self.lines.join("\n");
        all.push_str(
            "\n\n## Every command run on the far machine, in order, with how it ended\n\n",
        );
        all.push_str(&transcript::kept().join("\n"));
        all.push('\n');
        let _ = std::fs::write(self.folder.join("receipt.txt"), all);
    }
}

/// One reading command on the far machine: its output, where it ended
/// well.
fn read(link: &Link, script: &str) -> Option<String> {
    let ran = link.run(&FarCommand {
        script,
        args: &[],
        stdin: None,
        limit: Duration::from_secs(30),
        most_output: 64 * 1024,
    });
    match ran {
        Ok(ran) if ran.end == End::Ended(0) => {
            Some(String::from_utf8_lossy(&ran.stdout).trim().to_owned())
        }
        _ => None,
    }
}

/// The sockets in the control folder: one for each master that is up.
fn sockets(folder: &Folder) -> Vec<String> {
    let plain = |name: &String| !name.ends_with(".pid") && !name.ends_with(".log");
    lab::entries(folder.path())
        .into_iter()
        .filter(plain)
        .collect()
}

/// Every `ssh` process that names a socket in `folder`.
fn masters(folder: &Folder) -> Vec<String> {
    let listed = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,args="])
        .output();
    let marker = format!("ControlPath=\"{}", folder.path().display());
    let said = String::from_utf8_lossy(&listed.expect("ps").stdout).into_owned();
    let ours = |line: &&str| line.contains(&marker);
    said.lines().filter(ours).map(str::to_owned).collect()
}

/// The hashes of every profile's `config.yaml` on the far machine.
fn config_hashes(link: &Link) -> Option<String> {
    let script = format!(
        "root=\"{ROOT}\"; for file in \"$root/config.yaml\" \"$root\"/profiles/*/config.yaml; \
         do [ -f \"$file\" ] && shasum -a 256 \"$file\"; done; exit 0"
    );
    read(link, &script)
}

fn summed(mut times: Vec<u64>) -> String {
    times.sort();
    let at = |part: usize| times[(times.len() * part / 100).min(times.len() - 1)];
    let within = times.iter().filter(|ms| **ms <= 1000).count();
    format!(
        "n={} least_ms={} middle_ms={} p95_ms={} most_ms={} within_1s={within}",
        times.len(),
        times[0],
        at(50),
        at(95),
        times[times.len() - 1]
    )
}

/// What the log says each poll of each gateway took, and how many calls
/// and snapshots it made.
fn polls(log: &str) -> BTreeMap<String, Vec<(u64, u64, u64)>> {
    let mut by_reader: BTreeMap<String, Vec<(u64, u64, u64)>> = BTreeMap::new();
    for line in log.lines() {
        let Some((before, after)) = line.split_once(" trace poll ms=") else {
            continue;
        };
        let reader = before.split(' ').nth(1).unwrap_or("").to_owned();
        let number = |name: &str| {
            let found = after.split(' ').find_map(|part| part.strip_prefix(name));
            found.and_then(|text| text.parse::<u64>().ok()).unwrap_or(0)
        };
        let ms = after
            .split(' ')
            .next()
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(0);
        let poll = (ms, number("calls="), number("snapshots="));
        by_reader.entry(reader).or_default().push(poll);
    }
    by_reader
}

#[test]
#[ignore = "reads a real far machine: run by hand"]
fn a_real_far_machine_is_followed_and_only_read() {
    let (Ok(host), Ok(out)) = (
        std::env::var("AGENT_HUD_FAR_HOST"),
        std::env::var("AGENT_HUD_FAR_RECEIPT"),
    ) else {
        eprintln!("skipped: AGENT_HUD_FAR_HOST and AGENT_HUD_FAR_RECEIPT name no machine");
        return;
    };
    tap::read_only();
    transcript::keep();
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let mut receipt = Receipt {
        folder: out.clone(),
        lines: Vec::new(),
    };
    // A short folder of this test's own for the masters and the data.
    let temp = tempfile::Builder::new()
        .prefix("ahm")
        .tempdir_in(lab::short_base())
        .unwrap();
    let folder = Folder::open(&temp.path().join("c")).expect("the control folder");
    let data = temp.path().join("data");
    crate::os::private_directory(&data).unwrap();
    let settings = Settings {
        connect_timeout: Duration::from_secs(60),
        ..Settings::system()
    };
    let target = Target::new(&host, None, None, None).expect("a plain host");
    receipt.say(format!(
        "# A real far machine, read only: host alias `{host}`"
    ));
    receipt.say(format!("control folder: {}", folder.path().display()));
    let mut seen_sockets: Vec<String> = Vec::new();

    // Before anything else: free disk, the config hashes, the install id.
    let began = Instant::now();
    let link = match Link::open(&folder, &target, &settings) {
        Ok(link) => link,
        Err(failure) => {
            receipt.say(format!("STOP: the link did not open: {failure}"));
            panic!("the link did not open: {failure}");
        }
    };
    receipt.say(format!(
        "first link up after {} ms",
        began.elapsed().as_millis()
    ));
    seen_sockets.extend(sockets(&folder));
    let free = read(&link, "df -Pk \"$HOME\" | tail -1 | awk '{print $4}'");
    let free_kb: u64 = free.and_then(|kb| kb.parse().ok()).unwrap_or(0);
    receipt.say(format!(
        "free disk: {:.1} GB",
        free_kb as f64 / 1024.0 / 1024.0
    ));
    if free_kb < LEAST_FREE_KB {
        link.close();
        receipt.say("STOP: under 5 GB free");
        panic!("under 5 GB free on the far machine");
    }
    let before = config_hashes(&link).unwrap_or_default();
    receipt.say(format!("config.yaml hashes before:\n{before}"));
    let install = read(
        &link,
        &format!("[ -s \"{ROOT}/install_id\" ] && echo present || echo absent"),
    );
    receipt.say(format!("install id file: {install:?}"));
    if install.as_deref() != Some("present") {
        // Asking a Hermes without one for its id makes it write one.
        link.close();
        receipt.say("STOP: Hermes there has no install id yet, and asking would make it write one");
        panic!("no install id on the far machine");
    }
    // Where `python3` is, and whether the developer tools are there. It
    // is not run here: without the tools `/usr/bin/python3` is a stub
    // that opens an install window. The homes command checks the same
    // before it runs it.
    let python = read(
        &link,
        "command -v python3; /usr/bin/xcode-select -p 2>/dev/null || echo no-developer-tools",
    );
    receipt.say(format!("python3 there: {:?}", python.unwrap_or_default()));
    let closed = link.close();
    receipt.say(format!("first link closed: {closed:?}"));

    // Followed, at the app's own pace, with every poll said in the log.
    let requests = Requests::default();
    let entry = FarEntry {
        target: target.clone(),
        label: "Mac Mini".to_owned(),
        folder: folder.clone(),
        settings: settings.clone(),
    };
    let pace = Pace {
        trace: true,
        ..Pace::default()
    };
    let wake = Arc::new(|| {});
    let began = Instant::now();
    let observer = FarObserver::start_at(entry, &data, requests.clone(), wake, pace).unwrap();
    let log = || std::fs::read_to_string(data.join("hermes-gateway.log")).unwrap_or_default();
    let mut standing = Standing::Connecting;
    while !matches!(standing, Standing::Connected(_)) && began.elapsed() < Duration::from_secs(240)
    {
        let now = observer.standing();
        if now != standing {
            receipt.say(format!("{} ms: {now:?}", began.elapsed().as_millis()));
            standing = now;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    seen_sockets.extend(sockets(&folder));
    let followed = matches!(standing, Standing::Connected(_));
    if !followed {
        receipt.say(format!(
            "NOT FOLLOWED: {standing:?}; not followed: {:?}",
            observer.not_followed()
        ));
        receipt.say(format!("log:\n{}", log()));
    }

    let mut ready = 0;
    if followed {
        let followers = observer.inner().followers().expect("the followers");
        // Every gateway found is ready, or says why not, within a minute.
        let settled = Instant::now();
        loop {
            let set = followers.set().clone();
            ready = (set.iter())
                .filter(|one| matches!(one.model().gateway.connection(), Connection::Ready(_)))
                .count();
            if (ready >= 2 && ready == set.len()) || settled.elapsed() > Duration::from_secs(60) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // The natural pace for a while, then a poll asked for each
        // second, as a hook's report does, until enough are timed.
        std::thread::sleep(Duration::from_secs(30));
        let natural = polls(&log());
        for (reader, polled) in &natural {
            receipt.say(format!(
                "{reader}: {} polls in the first 30 s at its own pace",
                polled.len()
            ));
        }
        let limit = Instant::now() + Duration::from_secs(900);
        let enough = |log: &str| {
            let polled = polls(log);
            !polled.is_empty() && polled.values().all(|polled| polled.len() >= POLLS)
        };
        while !enough(&log()) && Instant::now() < limit {
            for follower in followers.set().iter() {
                follower.want_poll();
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        for follower in followers.set().iter() {
            let model = follower.model();
            let served = match model.gateway.connection() {
                Connection::Ready(instance) => {
                    let names: Vec<&str> = instance
                        .served
                        .iter()
                        .map(|served| served.profile.name())
                        .collect();
                    let version = instance.version.as_deref().unwrap_or("?");
                    format!("Ready, serves [{}], Hermes {version}", names.join(","))
                }
                other => format!("{other:?}"),
            };
            let mut live: BTreeMap<String, usize> = BTreeMap::new();
            for (_, status) in model.gateway.statuses() {
                *live.entry(format!("{status:?}")).or_default() += 1;
            }
            let cards = model.gateway.waiting().count();
            receipt.say(format!(
                "gateway {}: {served}; live sessions listed: {live:?}; requests waiting: {cards}",
                follower.reader
            ));
        }
        for (reader, polled) in polls(&log()) {
            let times: Vec<u64> = polled.iter().map(|poll| poll.0).collect();
            let calls: Vec<u64> = polled.iter().map(|poll| poll.1).collect();
            let snapshots: u64 = polled.iter().map(|poll| poll.2).sum();
            let (least, most) = (calls.iter().min().unwrap(), calls.iter().max().unwrap());
            receipt.say(format!(
                "POLLS {reader}: {} calls_per_poll={least}..{most} snapshots={snapshots}",
                summed(times)
            ));
        }
    }
    let cards = requests.views(crate::hooks::now_ms());
    let kinds: Vec<&str> = cards
        .iter()
        .filter_map(|card| card["kind"].as_str())
        .collect();
    receipt.say(format!(
        "cards in this test's own requests, none answered: {kinds:?}"
    ));
    let mut sent: BTreeMap<String, usize> = BTreeMap::new();
    for method in tap::all_written() {
        *sent.entry(method).or_default() += 1;
    }
    receipt.say(format!(
        "every frame written to the far gateways, by method: {sent:?}"
    ));
    let _ = std::fs::write(out.join("hermes-gateway.log"), log());
    drop(observer);
    receipt.say(format!(
        "observer stopped; masters left: {:?}",
        masters(&folder)
    ));

    // After: the same hashes, over a link of its own.
    let link = Link::open(&folder, &target, &settings).expect("the last link");
    seen_sockets.extend(sockets(&folder));
    let after = config_hashes(&link).unwrap_or_default();
    receipt.say(format!("config.yaml hashes after:\n{after}"));
    let closed = link.close();
    receipt.say(format!("last link closed: {closed:?}"));

    // The end: nothing answers on any control path, and no `ssh` is left.
    seen_sockets.sort();
    seen_sockets.dedup();
    let mut answering = 0;
    for name in &seen_sockets {
        let control = folder.path().join(name);
        let check = Command::new("/usr/bin/ssh")
            .args(["-F", "none", "-o"])
            .arg(format!("ControlPath={}", control.display()))
            .args(["-O", "check", "--", "link.invalid"])
            .output()
            .expect("ssh");
        let said = String::from_utf8_lossy(&check.stderr);
        let said = said.lines().next().unwrap_or("").to_owned();
        receipt.say(format!(
            "ssh -O check on {}: exit {:?}: {said}",
            control.display(),
            check.status.code()
        ));
        answering += usize::from(check.status.success());
    }
    let left = masters(&folder);
    receipt.say(format!("ssh processes naming the control folder: {left:?}"));
    receipt.say(format!(
        "entries left in the control folder: {:?}",
        lab::entries(folder.path())
    ));

    // Nothing that subscribes or answers was written.
    let forbidden = [
        "session.resume",
        "session.detach",
        "clarify.respond",
        "approval.respond",
        "a result frame",
    ];
    let wrote = |method: &&str| sent.contains_key(*method);
    assert!(!forbidden.iter().any(wrote), "{sent:?}");
    assert_eq!(answering, 0, "a master still answers");
    assert_eq!(left, Vec::<String>::new(), "an ssh process is left");
    assert_eq!(
        seen_sockets.len(),
        3,
        "one control path for each of the three links"
    );
    assert!(
        !before.is_empty() && before == after,
        "the config hashes differ"
    );
    assert!(followed, "the machine was not followed: {standing:?}");
    assert!(ready >= 2, "{ready} gateways ready");
}
