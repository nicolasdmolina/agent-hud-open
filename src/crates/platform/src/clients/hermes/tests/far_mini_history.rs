//! A real far machine's history, measured and synced once, and only
//! read (ticket 37). Run by hand, against a host of the user's own `ssh`
//! configuration:
//! `AGENT_HUD_FAR_HOST=<host> AGENT_HUD_FAR_RECEIPT=<folder> cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_mini_history -- --ignored --nocapture`.
//!
//! Like `far_mini.rs` it writes none of the user's `ssh` files, and its
//! master is its own. On the far machine it runs commands that read
//! (free disk, the hashes of every profile's `config.yaml`, the names and
//! sizes of the files in each Hermes home, a count of the rows of each
//! `state.db` through a connection that cannot write, the process list),
//! then what the app runs: the homes command, the relay and the history
//! reader. Over the gateways' WebSockets it sends only reads, and
//! nothing answers a card. The mirrors it fills are in a folder of this
//! test's own, which is removed with it.
use super::super::{
    far_history::{Reader, Settings},
    history::Plan,
    route::Route,
    tap, FarEntry, FarObserver, Pace, Standing,
};
use super::far_mini::{config_hashes, masters, read, summed, Receipt, LEAST_FREE_KB, ROOT};
use crate::ssh_link::{
    unix::tests::{lab, transcript},
    End, FarCommand, Folder, Link, Settings as LinkSettings, Target,
};
use agent_hud_agents::hermes::{
    far_history::Request,
    gateway::{Listed, Listing, Profile},
    mirror,
};
use agent_hud_core::{requests::Requests, AgentHudCore, SourceRoot};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// How many times each kind of read is timed.
const SAMPLES: usize = 5;

/// This machine's load, for every number that is a time.
fn load() -> String {
    let said = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "vm.loadavg"])
        .output();
    said.map_or_else(
        |_| "?".into(),
        |out| String::from_utf8_lossy(&out.stdout).trim().to_owned(),
    )
}

/// The names and sizes of the files in each Hermes home there, one
/// level deep, with each file's time of change.
fn listing(link: &Link) -> Option<String> {
    let script = format!(
        "root=\"{ROOT}\"; for home in \"$root\" \"$root\"/profiles/*; do [ -d \"$home\" ] || continue; \
         echo \"## $home\"; for file in \"$home\"/* \"$home\"/.[!.]*; do \
         [ -f \"$file\" ] && stat -f '%z %m %N' \"$file\"; done; done; exit 0"
    );
    read(link, &script)
}

/// The far processes that carry a history reader's mark.
fn far_readers(link: &Link) -> Option<String> {
    read(
        link,
        "ps -A -ww -o pid=,args= | grep 'agent-hud-far-history:' | grep -v grep; exit 0",
    )
}

/// The rows of the `state.db` in `home`, counted there through a
/// connection that cannot write: sessions, usage rows.
fn far_counts(link: &Link, home: &str) -> Option<(i64, i64)> {
    let code = "import sqlite3,sys; db=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True); \
        print(db.execute('select count(*) from sessions').fetchone()[0], \
        db.execute('select count(*) from session_model_usage').fetchone()[0])";
    let path = format!("{home}/state.db");
    let ran = link.run(&FarCommand {
        script: "exec python3 -I -c \"$1\" \"$2\"",
        args: &[code, &path],
        stdin: None,
        limit: Duration::from_secs(60),
        most_output: 4096,
    });
    let said = match ran {
        Ok(ran) if ran.end == End::Ended(0) => String::from_utf8_lossy(&ran.stdout).into_owned(),
        _ => return None,
    };
    let mut counts = said.split_whitespace().map(str::parse::<i64>);
    Some((counts.next()?.ok()?, counts.next()?.ok()?))
}

/// The rows a mirror holds: durable sessions, usage rows.
fn mirror_counts(path: &Path) -> (i64, i64) {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let Ok(db) = rusqlite::Connection::open_with_flags(path, flags) else {
        return (0, 0);
    };
    let count = |sql: &str| db.query_row(sql, [], |row| row.get(0)).unwrap_or(0);
    (
        count("SELECT COUNT(*) FROM sessions s JOIN hud_rows r ON r.session_id=s.id WHERE r.durable=1"),
        count("SELECT COUNT(*) FROM session_model_usage"),
    )
}

/// What the log says each poll took, from its line `from` on.
fn poll_ms(log: &str, from: usize) -> Vec<u64> {
    let took = |line: &str| {
        let ms = line.split(" trace poll ms=").nth(1)?.split(' ').next()?;
        ms.parse().ok()
    };
    log.lines().skip(from).filter_map(took).collect()
}

fn run(receipt: &mut Receipt, folder: &Folder, data: &Path, host: &str) -> Result<(), String> {
    let out = receipt.folder.clone();
    let settings = LinkSettings {
        connect_timeout: Duration::from_secs(60),
        ..LinkSettings::system()
    };
    let target = Target::new(host, None, None, None).map_err(|why| why.reason().to_owned())?;
    receipt.say(format!("this Mac's load at the start: {}", load()));

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
    let files_before = listing(&link).unwrap_or_default();
    let _ = std::fs::write(out.join("homes-before.txt"), &files_before);
    receipt.say(format!("far readers before: {:?}", far_readers(&link)));
    let install = read(
        &link,
        &format!("[ -s \"{ROOT}/install_id\" ] && echo present || echo absent"),
    );
    if install.as_deref() != Some("present") {
        receipt.say(format!("first link closed: {:?}", link.close()));
        return Err("STOP: Hermes there has no install id".into());
    }
    receipt.say(format!("first link closed: {:?}", link.close()));

    // Followed at the app's own pace, every poll said in the log, and
    // the history refresh ticked by hand.
    let requests = Requests::default();
    let entry = FarEntry {
        target: target.clone(),
        label: "Mac Mini".to_owned(),
        folder: folder.clone(),
        settings: settings.clone(),
    };
    let plan = Plan {
        clock: false,
        ..Plan::default()
    };
    let pace = Pace {
        trace: true,
        history: plan,
        ..Pace::default()
    };
    let observer = FarObserver::start_at(entry, data, requests.clone(), Arc::new(|| {}), pace)?;
    let log = || std::fs::read_to_string(data.join("hermes-gateway.log")).unwrap_or_default();
    let began = Instant::now();
    let mut key = None;
    while key.is_none() && began.elapsed() < Duration::from_secs(240) {
        if let Standing::Connected(found) = observer.standing() {
            key = Some(found);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    receipt.note_paths(folder);
    let Some(key) = key else {
        receipt.say(format!(
            "NOT FOLLOWED: {:?}\n{}",
            observer.standing(),
            log()
        ));
        return Err("the machine was not followed".into());
    };
    receipt.say(format!("followed after {} ms", began.elapsed().as_millis()));
    let history = (observer.book().history_of("given")).ok_or("the machine has no refresh")?;
    let followers = observer.inner().followers().ok_or("no followers")?;
    let route = observer.inner().route.clone();
    let link = route.link().ok_or("no link")?.0;
    let homes: Vec<(Profile, String)> = (route.inventory().map_err(|_| "no homes")?.homes.iter())
        .map(|home| (home.profile().clone(), route.named(home).into_owned()))
        .collect();
    let names: Vec<&str> = homes.iter().map(|home| home.0.name()).collect();
    receipt.say(format!("Hermes homes there: {names:?}"));
    let mut counted = BTreeMap::new();
    for (profile, home) in &homes {
        let counts = far_counts(&link, home);
        receipt.say(format!(
            "COUNT before, on the far machine, {}: sessions, usage rows = {counts:?}",
            profile.name()
        ));
        counted.insert(profile.clone(), counts);
    }

    // A poll asked for each second from here to the end, as a hook's
    // report does: how long each takes is what a card waits on.
    let polling = Arc::new(AtomicBool::new(true));
    let (asked, asking) = (polling.clone(), followers.clone());
    let nudges = std::thread::spawn(move || {
        while asked.load(Ordering::Acquire) {
            for follower in asking.set().iter() {
                follower.want_poll();
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    std::thread::sleep(Duration::from_secs(30));
    let quiet = poll_ms(&log(), 0);
    receipt.say(format!(
        "POLLS with no history read, load {}: {}",
        load(),
        summed(quiet)
    ));
    let reads_from = log().lines().count();

    // The measurement: each kind of read, five times, on every home
    // that has a database. Each line is the reader's own, with its
    // exit status, its time and its bytes.
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let kept = lines.clone();
    let reader = Reader::new(Settings::default(), move |line| {
        kept.lock().unwrap().push(line.to_owned());
    });
    let slot = history.slot().clone();
    receipt.say(format!("## The measurement, load {}", load()));
    for (profile, home) in &homes {
        let name = profile.name();
        let page = reader.prints(&link, &slot, home, 0, plan.page, None);
        let ids: Vec<String> = match &page {
            Ok(page) => (page
                .prints
                .iter()
                .take(plan.batch)
                .map(|print| print.id.clone()))
            .collect(),
            Err(failure) => {
                receipt.say(format!("MEASURE {name}: no fingerprint page: {failure}"));
                Vec::new()
            }
        };
        let since = (crate::hooks::now_ms() / 1000 - 7 * 86_400) as f64;
        for _ in 0..SAMPLES {
            let _ = reader.read(&link, &slot, home, &Request::Probe);
            let _ = reader.prints(&link, &slot, home, 0, plan.page, None);
            let _ = reader.prints(&link, &slot, home, 0, plan.page, Some(since));
            if !ids.is_empty() {
                let _ = reader.sessions(&link, &slot, home, &ids);
            }
            let _ = reader.held(&link, &slot, home);
        }
        for line in std::mem::take(&mut *lines.lock().unwrap()) {
            receipt.say(format!("MEASURE {name}: {line}"));
        }
    }

    // One whole first sync of every profile, tick after tick, with the
    // refresh as the app runs it but for the wait between two ticks.
    receipt.say(format!("## The first sync, load {}", load()));
    let (began, mut ticks, mut reads) = (Instant::now(), 0, 0);
    let (mut took, mut synced, mut stalled) = (Vec::new(), 0, 0);
    let root = data.join(mirror::ROOT);
    loop {
        let tick = Instant::now();
        let ticked = history.tick();
        took.push(tick.elapsed().as_millis() as u64);
        (ticks, reads) = (ticks + 1, reads + ticked.reads);
        if ticked.reads > 0 && !ticked.overdue {
            break;
        }
        // A home that cannot be read stays overdue: once three ticks in
        // a row wrote no session, the sync has done what it can.
        let held: i64 = (homes.iter())
            .map(|(profile, _)| mirror_counts(&mirror::file(&root, &key, profile)).0)
            .sum();
        stalled = if held == synced { stalled + 1 } else { 0 };
        synced = held;
        if stalled >= 3 || ticks >= 200 || began.elapsed() > Duration::from_secs(2_400) {
            receipt.say("THE FIRST SYNC DID NOT FINISH FOR EVERY HOME");
            break;
        }
    }
    receipt.say(format!(
        "first sync: {ticks} ticks, {reads} reads, {} s in all; a tick took {}",
        began.elapsed().as_secs(),
        summed(took)
    ));
    // Then ticks with nothing new to read: what a tick costs from then on.
    let mut steady = Vec::new();
    for _ in 0..SAMPLES {
        let tick = Instant::now();
        let ticked = history.tick();
        steady.push((tick.elapsed().as_millis() as u64, ticked.reads));
    }
    receipt.say(format!(
        "ticks after the sync (ms, reads), load {}: {steady:?}",
        load()
    ));
    receipt.say(format!("the machine's ages: {:?}", history.ages()));
    for line in log()
        .lines()
        .filter(|line| line.contains("history stopped"))
    {
        receipt.say(format!("STOPPED {line}"));
    }
    let during = poll_ms(&log(), reads_from);
    receipt.say(format!(
        "POLLS while history reads ran back to back, load {}: {}",
        load(),
        summed(during)
    ));
    polling.store(false, Ordering::Release);
    let _ = nudges.join();

    // The count comparison: each mirror against the far database it
    // was read from, counted there again now.
    receipt.say("## The count comparison");
    let mut matched = true;
    for (profile, home) in &homes {
        let name = profile.name();
        let (first, now) = (counted[profile], far_counts(&link, home));
        let held = mirror_counts(&mirror::file(&root, &key, profile));
        receipt.say(format!(
            "COUNT {name}: on the far machine before {first:?}, after {now:?}; in the mirror (sessions, usage rows) {held:?}"
        ));
        let (Some(first), Some(now)) = (first, now) else {
            continue;
        };
        // The far Hermes runs: rows are added while this reads.
        let within =
            |held: i64, first: i64, now: i64| held >= first.min(now) && held <= first.max(now);
        let same = within(held.0, first.0, now.0) && within(held.1, first.1, now.1);
        let rows = held.0 > 0 || now.0 == 0;
        receipt.say(format!("COUNT {name}: match={same} rows_came={rows}"));
        matched &= same && rows;
    }
    let skipped = log()
        .lines()
        .filter(|line| line.contains("skips one session"))
        .count();
    receipt.say(format!("sessions skipped whole by the reader: {skipped}"));

    // A session that begins there, with the synced mirrors behind it:
    // how long core takes to list a new row once the writer told.
    let source = SourceRoot {
        path: root.clone(),
        provider: "hermes",
    };
    let mut core = AgentHudCore::new(vec![source], data.join("core"));
    let (reading, mut passes) = (Instant::now(), 1);
    core.request("refresh", json!({}))
        .map_err(|why| why.to_string())?;
    while core.indexing() && passes < 2_000 {
        core.file_changes(Some(HashSet::new()));
        core.request("snapshot", json!({}))
            .map_err(|why| why.to_string())?;
        passes += 1;
    }
    let total = |core: &mut AgentHudCore| {
        let found = core.request("sessions", json!({"scope": "all", "search": ""}));
        found.map_or(0, |found| found["total"].as_u64().unwrap_or(0))
    };
    receipt.say(format!(
        "core read the mirrors in {passes} passes, {} ms: {} sessions listed",
        reading.elapsed().as_millis(),
        total(&mut core)
    ));
    if let Some(writer) = &followers.mirror {
        let said = Listed {
            title: Some("A made-up row, in this test's own mirror".into()),
            ..Listed::default()
        };
        let rows = vec![("hud-test-made-up".to_owned(), said)];
        let listing = Listing {
            read_ms: crate::hooks::now_ms(),
            profiles: BTreeMap::from([(Profile::Default, rows)]),
            samples: BTreeMap::new(),
        };
        let before = total(&mut core);
        writer.listed(listing, route.number().max(1));
        writer.settle();
        let committed = Instant::now();
        core.written_files(HashSet::from([mirror::file(
            &root,
            &key,
            &Profile::Default,
        )]));
        core.request("snapshot", json!({}))
            .map_err(|why| why.to_string())?;
        let listed = total(&mut core) == before + 1;
        receipt.say(format!(
            "a new row in the synced mirror: listed at the first pass = {listed}, {} ms after its commit, load {}",
            committed.elapsed().as_millis(),
            load()
        ));
    }
    drop(core);

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
    let _ = std::fs::write(out.join("hermes-gateway.log"), log());
    drop((link, history, followers, route));
    drop(observer);
    receipt.say(format!(
        "observer stopped; masters left: {:?}",
        masters(folder)
    ));

    // After: no reader left there, the same hashes, the same files.
    let link = Link::open(folder, &target, &settings)
        .map_err(|failure| format!("the last link did not open: {failure}"))?;
    receipt.note_paths(folder);
    let readers = far_readers(&link);
    receipt.say(format!("far readers after: {readers:?}"));
    let after = config_hashes(&link).unwrap_or_default();
    receipt.say(format!("config.yaml hashes after:\n{after}"));
    let files_after = listing(&link).unwrap_or_default();
    let _ = std::fs::write(out.join("homes-after.txt"), &files_after);
    let names = |text: &str| -> HashSet<String> {
        let name = |line: &str| line.splitn(3, ' ').nth(2).map(str::to_owned);
        text.lines().filter_map(name).collect()
    };
    let (was, is) = (names(&files_before), names(&files_after));
    let mut new: Vec<&String> = is.difference(&was).collect();
    let mut gone: Vec<&String> = was.difference(&is).collect();
    new.sort();
    gone.sort();
    receipt.say(format!("files there now that were not before: {new:?}"));
    receipt.say(format!("files there before that are not now: {gone:?}"));
    receipt.say(format!("last link closed: {:?}", link.close()));
    receipt.say(format!("this Mac's load at the end: {}", load()));

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
    if !matched {
        return Err("the mirror's counts are not the far machine's".to_owned());
    }
    Ok(())
}

#[test]
#[ignore = "reads a real far machine: run by hand"]
fn a_real_far_machines_history_is_measured_and_synced() {
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
        "# A real far machine's history, read only: host alias `{host}`"
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
