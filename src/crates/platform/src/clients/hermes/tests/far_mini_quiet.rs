//! A real far machine whose gateway lists a session as working for
//! days (ticket 67), followed at the app's own pace and only read. Run
//! by hand, against a host of the user's own `ssh` configuration:
//! `AGENT_HUD_FAR_HOST=<host> AGENT_HUD_FAR_RECEIPT=<folder> cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_mini_quiet -- --ignored --nocapture`.
//!
//! It runs for the app's quiet time (thirty minutes), two minutes more,
//! and then `AGENT_HUD_FAR_WATCH_SECONDS` (fourteen minutes where that
//! is not set), and says, minute by minute, how many polls each gateway
//! was asked and when each history tick began.
//!
//! Like `far_mini.rs` it writes none of the user's `ssh` files, and its
//! master is its own. On the far machine it runs commands that read
//! (free disk, the hashes of every profile's `config.yaml`, the process
//! list), then what the app runs: the homes command, the relay and the
//! history reader. Over the gateways' WebSockets it sends only reads:
//! no visit begins (`tap.rs`), so nothing subscribes and nothing answers
//! a card. The mirrors it fills are in a folder of this test's own,
//! which is removed with it.
use super::super::{tap, FarEntry, FarObserver, Pace, Standing};
use super::far_mini::{config_hashes, masters, read, Receipt, LEAST_FREE_KB, ROOT};
use crate::ssh_link::{
    unix::tests::{lab, transcript},
    Folder, Link, Settings as LinkSettings, Target,
};
use agent_hud_agents::hermes::gateway::Status;
use agent_hud_core::requests::Requests;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// How long the watch lasts once the quiet time and the two minutes a
/// turn keeps the history's quick pace have passed: two slow ticks.
const WATCH: Duration = Duration::from_secs(14 * 60);
/// Two history reads further apart than this are of two ticks.
const TICK_GAP_MS: i64 = 20_000;

/// The far processes that carry a history reader's mark.
fn far_readers(link: &Link) -> Option<String> {
    read(
        link,
        "ps -A -ww -o pid=,args= | grep 'agent-hud-far-history:' | grep -v grep; exit 0",
    )
}

/// The time of day a log line was written, in milliseconds.
fn at_ms(line: &str) -> Option<i64> {
    let time = line.split('T').nth(1)?.split('Z').next()?;
    let mut parts = time.split([':', '.']).map(str::parse::<i64>);
    let (h, m, s) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let ms = parts.next().and_then(Result::ok).unwrap_or(0);
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// What the log says of the polls, minute by minute from `began_ms`:
/// for each gateway, how many polls ran and how many calls they made.
fn by_minute(log: &str, began_ms: i64) -> BTreeMap<i64, BTreeMap<String, (u64, u64)>> {
    let mut minutes: BTreeMap<i64, BTreeMap<String, (u64, u64)>> = BTreeMap::new();
    for line in log.lines().filter(|line| line.contains(" trace poll ms=")) {
        let (Some(at), Some(gateway)) = (at_ms(line), line.split(' ').nth(1)) else {
            continue;
        };
        let calls = line.split(" calls=").nth(1);
        let calls = calls.and_then(|rest| rest.split(' ').next()?.parse::<u64>().ok());
        let name = gateway.split('/').nth(1).unwrap_or(gateway).to_owned();
        let of = minutes
            .entry((at - began_ms).div_euclid(60_000))
            .or_default();
        let sum = of.entry(name).or_default();
        *sum = (sum.0 + 1, sum.1 + calls.unwrap_or(0));
    }
    minutes
}

/// When each history tick began, in seconds from `began_ms`, with how
/// many far reads it made.
fn ticks(log: &str, began_ms: i64) -> Vec<(i64, usize)> {
    let read = |line: &&str| {
        let reader = line.split(' ').nth(1) == Some("history");
        reader && !line.contains(" probes first")
    };
    let mut all: Vec<(i64, usize)> = Vec::new();
    let mut last = i64::MIN;
    for at in log.lines().filter(read).filter_map(at_ms) {
        match all.last_mut() {
            Some(tick) if at - last <= TICK_GAP_MS => tick.1 += 1,
            _ => all.push(((at - began_ms) / 1000, 1)),
        }
        last = at;
    }
    all
}

fn run(receipt: &mut Receipt, folder: &Folder, data: &Path, host: &str) -> Result<(), String> {
    let out = receipt.folder.clone();
    let settings = LinkSettings {
        connect_timeout: Duration::from_secs(60),
        ..LinkSettings::system()
    };
    let target = Target::new(host, None, None, None).map_err(|why| why.reason().to_owned())?;

    // Before anything else: free disk, then what must not change.
    let link = Link::open(folder, &target, &settings)
        .map_err(|failure| format!("STOP: the link did not open: {failure}"))?;
    receipt.note_paths(folder);
    let free = read(&link, "df -Pk \"$HOME\" | tail -1 | awk '{print $4}'");
    let free_kb: u64 = free.and_then(|kb| kb.parse().ok()).unwrap_or(0);
    receipt.say(format!(
        "free disk: {:.1} GB",
        free_kb as f64 / 1024.0 / 1024.0
    ));
    if free_kb < LEAST_FREE_KB {
        receipt.say(format!("first link closed: {:?}", link.close()));
        return Err("STOP: under 5 GB free on the far machine".to_owned());
    }
    let before = config_hashes(&link).unwrap_or_default();
    receipt.say(format!("config.yaml hashes before:\n{before}"));
    receipt.say(format!("far readers before: {:?}", far_readers(&link)));
    let install = read(
        &link,
        &format!("[ -s \"{ROOT}/install_id\" ] && echo present || echo absent"),
    );
    receipt.say(format!("first link closed: {:?}", link.close()));
    if install.as_deref() != Some("present") {
        return Err("STOP: Hermes there has no install id".into());
    }

    // Followed at the app's own pace, its history by the clock as in
    // the app. Every poll is said in the log, which changes no pace.
    let pace = Pace {
        trace: true,
        ..Pace::default()
    };
    let watch = std::env::var("AGENT_HUD_FAR_WATCH_SECONDS");
    let watch = watch.ok().and_then(|seconds| seconds.parse().ok());
    let watch = watch.map_or(WATCH, Duration::from_secs);
    let length = pace.quiet + pace.history.worked + watch;
    receipt.say(format!(
        "the app's pace: quiet after {} s, close watch {} ms, history every {} s while a session works and {} s otherwise; this run lasts {} s",
        pace.quiet.as_secs(),
        pace.watch.as_millis(),
        pace.history.live.as_secs(),
        pace.history.idle.as_secs(),
        length.as_secs()
    ));
    let requests = Requests::default();
    let entry = FarEntry {
        target: target.clone(),
        label: "Mac Mini".to_owned(),
        folder: folder.clone(),
        settings: settings.clone(),
    };
    let observer = FarObserver::start_at(entry, data, requests.clone(), Arc::new(|| {}), pace)?;
    let log = || std::fs::read_to_string(data.join("hermes-gateway.log")).unwrap_or_default();
    let began = Instant::now();
    let began_ms = (crate::hooks::now_ms()).rem_euclid(86_400_000);
    let mut followed = false;
    while !followed && began.elapsed() < Duration::from_secs(240) {
        followed = matches!(observer.standing(), Standing::Connected(_));
        std::thread::sleep(Duration::from_millis(50));
    }
    receipt.note_paths(folder);
    if !followed {
        receipt.say(format!(
            "NOT FOLLOWED: {:?}\n{}",
            observer.standing(),
            log()
        ));
        return Err("the machine was not followed".into());
    }
    receipt.say(format!("followed after {} ms", began.elapsed().as_millis()));
    let followers = observer.inner().followers().ok_or("no followers")?;

    // What each gateway lists and what its model says of the paces,
    // said when it changes: counts only, no session is named.
    let mut said = String::new();
    while began.elapsed() < length {
        let mut now = Vec::new();
        for follower in followers.set().iter() {
            let model = follower.model();
            let gateway = &model.gateway;
            let count = |status: Status| {
                let listed = gateway.statuses();
                listed.iter().filter(|(_, is)| *is == status).count()
            };
            now.push(format!(
                "{}: working {} starting {} waiting {} idle {}; poll_ms={} close_watch={} at_work={}",
                gateway.id().anchor().name(),
                count(Status::Working),
                count(Status::Starting),
                count(Status::Waiting),
                count(Status::Idle),
                gateway.poll_ms(),
                gateway.lively(),
                gateway.working()
            ));
        }
        now.sort();
        let now = now.join(" | ");
        if now != said {
            receipt.say(format!("at {} s: {now}", began.elapsed().as_secs()));
            said = now;
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    let text = log();
    receipt.say("## Polls, minute by minute: for each gateway, polls and the calls in them");
    for (minute, gateways) in by_minute(&text, began_ms) {
        let each: Vec<String> = (gateways.iter())
            .map(|(name, (polls, calls))| format!("{name} {polls} polls {calls} calls"))
            .collect();
        receipt.say(format!("minute {minute:>2}: {}", each.join(" | ")));
    }
    receipt.say("## The app's own cost lines");
    for line in text.lines().filter(|line| line.contains(" polls n=")) {
        receipt.say(line.to_owned());
    }
    receipt.say(
        "## History ticks: when each began, the reads in it, and how long after the one before",
    );
    let mut last = None;
    for (at, reads) in ticks(&text, began_ms) {
        let after = last.map_or_else(String::new, |last: i64| format!(", {} s after", at - last));
        receipt.say(format!("tick at {at} s: {reads} reads{after}"));
        last = Some(at);
    }
    for line in text.lines().filter(|line| line.contains("history stopped")) {
        receipt.say(format!("STOPPED {line}"));
    }
    let mut sent: BTreeMap<String, usize> = BTreeMap::new();
    for method in tap::all_written() {
        *sent.entry(method).or_default() += 1;
    }
    receipt.say(format!(
        "every frame written to the far gateways, by method: {sent:?}"
    ));
    let cards = requests.views(crate::hooks::now_ms()).len();
    receipt.say(format!(
        "cards in this test's own requests, none answered: {cards}"
    ));
    let _ = std::fs::write(out.join("hermes-gateway.log"), &text);
    drop(followers);
    drop(observer);
    receipt.say(format!(
        "observer stopped; masters left: {:?}",
        masters(folder)
    ));

    // After: no reader left there, and the same hashes.
    let link = Link::open(folder, &target, &settings)
        .map_err(|failure| format!("the last link did not open: {failure}"))?;
    receipt.note_paths(folder);
    let readers = far_readers(&link);
    receipt.say(format!("far readers after: {readers:?}"));
    let after = config_hashes(&link).unwrap_or_default();
    receipt.say(format!("config.yaml hashes after:\n{after}"));
    receipt.say(format!("last link closed: {:?}", link.close()));

    let forbidden = [
        "session.resume",
        "session.detach",
        "clarify.respond",
        "approval.respond",
        "a result frame",
    ];
    if forbidden.iter().any(|method| sent.contains_key(*method)) {
        return Err(format!(
            "a frame that subscribes or answers was written: {sent:?}"
        ));
    }
    if before.is_empty() || before != after {
        return Err("the config.yaml hashes differ, or were not read".to_owned());
    }
    if readers.as_deref() != Some("") {
        return Err(format!(
            "a far reader is left, or none could be looked for: {readers:?}"
        ));
    }
    Ok(())
}

#[test]
#[ignore = "reads a real far machine: run by hand"]
fn a_real_far_gateway_that_lists_a_session_as_working_for_days_is_left_at_the_slow_paces() {
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
    let _ = std::fs::remove_file(out.join("receipt.txt"));
    let mut receipt = Receipt {
        folder: out,
        paths: Vec::new(),
    };
    let temp = tempfile::Builder::new()
        .prefix("ahm")
        .tempdir_in(lab::short_base())
        .unwrap();
    let folder = Folder::open(&temp.path().join("c")).expect("the control folder");
    let data = temp.path().join("data");
    crate::os::private_directory(&data).unwrap();
    receipt.say(format!(
        "# A real far machine's paces while nothing moves there, read only: host alias `{host}`"
    ));
    receipt.say(format!("HUD control folder: {}", folder.path().display()));
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&mut receipt, &folder, &data, &host)
    }));
    let ended = match ran {
        Ok(Ok(())) => "the run ended as it should".to_owned(),
        Ok(Err(why)) => why,
        Err(_) => "the run panicked".to_owned(),
    };
    receipt.say(format!("## How the run ended: {ended}"));
    let clean = receipt.proof(&folder);
    receipt.say(format!("## Clean: {clean}"));
    assert!(clean, "something of the HUD's link is left");
    assert_eq!(ended, "the run ended as it should");
}
