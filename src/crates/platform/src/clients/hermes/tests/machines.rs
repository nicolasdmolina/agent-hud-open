//! The machines that follow the connection list (ticket 32), in a lab:
//! a list file of the test's own, one throwaway `sshd` on loopback for
//! each far host (`ssh_link/unix/tests/lab.rs`), and in each far home a
//! Hermes root with the pretend gateways of the other tests (`fake.rs`).
//! "Far" is this same machine signed in as the user who runs the tests,
//! with a home folder of the lab's own. This machine's Hermes is a
//! folder of the lab's too. Nothing here reads the user's connection
//! list, Hermes home or `ssh` files: every list, binding and host is
//! made up, and the only `ssh` configuration read is the lab's.
//!
//! Core is a real one, so what a test reads of the machines list, the
//! cards and the alerts is what a snapshot says.
//!
//! The tests start processes, so they are marked `#[ignore]` and run by
//! `the_lab_tests_pass` in a process of their own. To run them here:
//! `cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::machines:: -- --ignored --nocapture`.
use super::{
    super::{
        machines::{Book, Sources, Ways},
        route::local::Local,
        Machines, Pace,
    },
    fake::Fake,
    marked, quick,
};
use crate::hooks::{Changed, Told};
use crate::ssh_link::{
    unix::tests::lab::{self, short_base, Lab},
    Folder, Settings,
};
use agent_hud_agents::{hermes::AGENT, HookEvent};
use agent_hud_core::{requests::Requests, AgentHudCore};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[path = "machines/alerts.rs"]
mod alerts;
#[path = "machines/answers.rs"]
mod answers;
#[path = "machines/guards.rs"]
mod guards;
#[path = "machines/history.rs"]
pub(super) mod history;
#[path = "machines/history_more.rs"]
mod history_more;
#[path = "machines/lists.rs"]
mod lists;
#[path = "machines/mirrors.rs"]
pub(super) mod mirrors;
#[path = "machines/owners.rs"]
mod owners;
#[path = "machines/reasons.rs"]
mod reasons;

#[test]
fn the_lab_tests_pass() {
    lab::in_a_process_of_their_own("clients::hermes::tests_unix::machines::");
}

/// What sits in the list's `token` and `headers`, and must be nowhere
/// else.
pub(super) const MARKER: &str = "MARKER-7f3a9c41-never-kept";
/// What the Hermes of the first and of the second far host report as
/// their installs.
pub(super) const INSTALL: [&str; 2] = [
    "0123456789abcdef0123456789abcdef",
    "fedcba9876543210fedcba9876543210",
];
/// The label of the list's entry for this machine.
pub(super) const HERE: &str = "MacBook";

/// One far host: an `sshd`, and the Hermes root in its home folder.
pub(super) struct Host {
    pub lab: Lab,
    pub root: PathBuf,
}

pub(super) struct Fleet {
    pub hosts: Vec<Host>,
    /// The app's data folder.
    pub data: PathBuf,
    /// The connection list.
    pub list: PathBuf,
    /// The Hermes root of this machine.
    pub local: PathBuf,
    /// The folder every master is in.
    pub masters: PathBuf,
    /// The `known_hosts` of the host name `stranger`: empty.
    pub unknown_hosts: PathBuf,
    pub core: AgentHudCore,
    pub requests: Requests,
    /// The files the mirrors' writers said they wrote, which a turn
    /// hands to core (`machines/mirrors.rs`).
    pub told: Arc<Mutex<HashSet<PathBuf>>>,
    /// When a writer first said so since a test last asked
    /// (`far_rows_timing.rs`): the moment after its commit.
    pub told_at: Arc<Mutex<Option<Instant>>>,
    /// The turns the far followers said started or ended, which a turn
    /// of the collector hands to core as hook events
    /// (`machines/alerts.rs`); and every one handed over so far.
    pub heard: Arc<Mutex<Vec<HookEvent>>>,
    pub facts: Vec<HookEvent>,
    /// Core reads the mirrors, and its file watcher has nothing to say:
    /// it hears that no file changed before every pass.
    pub silent: bool,
    pub machines: Option<Machines>,
    settings: Settings,
    _dir: tempfile::TempDir,
}

pub(super) fn pace() -> Pace {
    Pace {
        discover: Some(Duration::from_millis(500)),
        answer: Duration::from_secs(6),
        ..quick()
    }
}

/// Waits for `met`, with a limit for a machine busy with other builds: a
/// sign-in and a relay are processes each time.
pub(super) fn until(what: &str, mut met: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(120);
    while !met() {
        assert!(Instant::now() < limit, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The entry of the list for this machine.
pub(super) fn here() -> Value {
    json!({"id": "local", "kind": "local", "label": HERE})
}

/// An `ssh` entry, with a token and a header as the desktop app keeps
/// them.
pub(super) fn ssh(id: &str, label: &str, host: &str) -> Value {
    json!({"id": id, "kind": "ssh", "label": label, "host": host,
        "token": {"encoding": "plain", "value": MARKER},
        "headers": {"authorization": MARKER}})
}

/// The list's text, as the desktop app writes one.
pub(super) fn text(entries: &[Value], token: &str) -> String {
    let list = json!({"version": 2, "primary": "local", "launchMode": "primary",
        "lastUsed": token, "token": {"encoding": "plain", "value": token},
        "connections": entries});
    serde_json::to_string_pretty(&list).unwrap()
}

/// A fleet of `hosts` far hosts, or None where this machine cannot run
/// an `sshd`. Host `n` goes by the names `m<n>` and `m<n>b`; `stranger`
/// is the first host under a name whose key is not known, and `gone` is
/// a name that leads to a port nothing listens on.
pub(super) fn fleet(hosts: usize) -> Option<Fleet> {
    let dir = tempfile::Builder::new()
        .prefix("ahm")
        .tempdir_in(short_base())
        .expect("the fleet's folder");
    let at = std::fs::canonicalize(dir.path()).expect("the fleet's folder");
    let mut far = Vec::new();
    for _ in 0..hosts.max(1) {
        // A far login's home is the lab's, and its path has a folder of
        // the lab first, empty unless a test puts a stand-in there.
        let path = "{lab}/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        let lab = Lab::start_with(&[("HOME", "{lab}/far"), ("PATH", path)])?;
        let home = std::fs::canonicalize(lab.at("")).unwrap().join("far");
        let root = home.join(".hermes");
        crate::os::private_directory(&home).unwrap();
        crate::os::private_directory(&root).unwrap();
        far.push(Host { lab, root });
    }
    let (data, local) = (at.join("data"), at.join("home"));
    for folder in [&data, &local, &at.join("Hermes")] {
        crate::os::private_directory(folder).unwrap();
    }
    let unknown_hosts = at.join("unknown_hosts");
    std::fs::write(&unknown_hosts, "").unwrap();
    let mut config = String::new();
    let block = |names: &str, lab: &Lab, known: &std::path::Path| {
        format!(
            "Host {names}\n  HostName 127.0.0.1\n  Port {}\n  UserKnownHostsFile \"{}\"\n  \
             IdentityFile \"{}\"\n",
            lab.port,
            known.display(),
            lab.at("id").display()
        )
    };
    for (n, host) in far.iter().enumerate() {
        let known = host.lab.at("known_hosts");
        config += &block(&format!("m{n} m{n}b"), &host.lab, &known);
    }
    config += &block("stranger", &far[0].lab, &unknown_hosts);
    // A name for the first host at a port where nothing listens.
    let nowhere = format!("Port {}", lab::closed_port());
    let known = far[0].lab.at("known_hosts");
    let gone = block("gone", &far[0].lab, &known);
    config += &gone.replace(&format!("Port {}", far[0].lab.port), &nowhere);
    config += "Host *\n  GlobalKnownHostsFile /dev/null\n  IdentitiesOnly yes\n  \
               IdentityAgent none\n";
    // In the fleet's own folder: a host can go away and the rest go on.
    std::fs::write(at.join("config"), &config).unwrap();
    let settings = Settings {
        config: Some(at.join("config")),
        ..far[0].lab.settings()
    };
    let mut core = AgentHudCore::new(vec![], data.join("core"));
    let sound = json!({"key": "sound", "value": true});
    core.request("set_setting", sound).unwrap();
    Some(Fleet {
        hosts: far,
        list: at.join("Hermes").join("connections.json"),
        masters: at.join("c"),
        unknown_hosts,
        requests: core.requests(),
        core,
        told: Arc::default(),
        told_at: Arc::default(),
        heard: Arc::default(),
        facts: Vec::new(),
        silent: false,
        data,
        local,
        machines: None,
        settings,
        _dir: dir,
    })
}

impl Host {
    /// A gateway in the host's Hermes root that reports `install`.
    pub(super) fn gateway(&self, instance: &str, install: &str) -> Fake {
        let gateway = Fake::start(&self.root, instance);
        gateway.world().install = Some(install.into());
        gateway
    }
}

impl Fleet {
    /// Writes the list as the desktop app does: whole, then moved into
    /// place.
    pub(super) fn write(&self, entries: &[Value]) {
        self.write_text(&text(entries, "token-1"));
    }

    pub(super) fn write_text(&self, text: &str) {
        let new = self.list.with_extension("new");
        std::fs::write(&new, text).unwrap();
        std::fs::rename(&new, &self.list).unwrap();
    }

    /// Starts following, as turning client hooks on does.
    pub(super) fn start(&mut self) {
        self.start_at(pace());
    }

    /// The same, at `pace`.
    pub(super) fn start_at(&mut self, pace: Pace) {
        let (this, local) = (Local::at(&self.local), Local::at(&self.local));
        let ways = Ways {
            folder: Folder::open(&self.masters).expect("the control folder"),
            settings: self.settings.clone(),
        };
        let sources = Sources {
            local: Arc::new(local),
            here: Some(Arc::new(move || this.key())),
            list: Some(self.list.clone()),
            ways: Ok(ways),
            look: Duration::from_millis(40),
            pace,
            changed: self.changed(),
            told: self.hears(),
        };
        let (requests, wake) = (self.requests.clone(), Arc::new(|| {}));
        let machines = Machines::start_with(sources, &self.data, requests, wake);
        self.machines = Some(machines.expect("the machines"));
    }

    /// What a mirror writer is given to say which file it wrote: the
    /// paths are kept, as the hooks keep them.
    pub(super) fn changed(&self) -> Changed {
        let (kept, at) = (self.told.clone(), self.told_at.clone());
        Arc::new(move |paths: &[PathBuf]| {
            at.lock().unwrap().get_or_insert_with(Instant::now);
            kept.lock().unwrap().extend(paths.iter().cloned());
        })
    }

    /// What a far follower is given to say a turn started or ended: the
    /// facts are kept, as the hooks keep them.
    pub(super) fn hears(&self) -> Told {
        let kept = self.heard.clone();
        Arc::new(move |event: HookEvent| kept.lock().unwrap().push(event))
    }

    /// Hands core what the far followers said since the last call, as
    /// the collector does at a channel's wake.
    pub(super) fn hear(&mut self) {
        let events = std::mem::take(&mut *self.heard.lock().unwrap());
        if !events.is_empty() {
            self.facts.extend(events.iter().cloned());
            self.core.ingest_hook_events(events);
        }
    }

    /// Stops following, as turning client hooks off does.
    pub(super) fn stop(&mut self) {
        self.machines.take();
    }

    pub(super) fn book(&self) -> &Arc<Book> {
        self.machines.as_ref().expect("following").book()
    }

    /// The route number of the one far machine there is.
    pub(super) fn number(&self) -> u64 {
        let far = self.book().far();
        assert_eq!(far.len(), 1, "{far:?}");
        far[0].2
    }

    /// Waits until the list was looked at `more` times from now.
    pub(super) fn looked(&self, more: u64) {
        let machines = self.machines.as_ref().expect("following");
        let from = machines.looks();
        until("the list is looked at", || machines.looks() >= from + more);
    }

    pub(super) fn snapshot(&mut self) -> Value {
        self.hear();
        if self.silent {
            self.core.file_changes(Some(HashSet::new()));
        }
        self.core.request("snapshot", json!({})).unwrap()
    }

    /// The machines list, as a snapshot has it.
    pub(super) fn rows(&mut self) -> Vec<Value> {
        let listed = self.snapshot()["machines"][AGENT.id].clone();
        listed.as_array().cloned().unwrap_or_default()
    }

    /// The status of the entry that goes by `label`.
    pub(super) fn row(&mut self, label: &str) -> Option<Value> {
        self.rows().into_iter().find(|row| row["label"] == label)
    }

    /// The note on the whole list.
    pub(super) fn note(&mut self) -> Option<String> {
        let note = self.snapshot()["machinesNote"][AGENT.id].clone();
        note.as_str().map(str::to_owned)
    }

    /// Waits until the entry `label` stands as `state`: its status.
    pub(super) fn stands(&mut self, label: &str, state: &str) -> Value {
        let mut found = None;
        until(&format!("{label} is {state}"), || {
            found = self.row(label).filter(|row| row["state"] == state);
            found.is_some()
        });
        found.unwrap()
    }

    /// Waits until the entry `label` is connected: its machine's key.
    pub(super) fn connected(&mut self, label: &str) -> String {
        let row = self.stands(label, "connected");
        row["id"].as_str().unwrap().to_owned()
    }

    /// Waits until the entry `label` is connected as another machine
    /// than `before`: that machine's key.
    pub(super) fn moved(&mut self, label: &str, before: &str) -> String {
        let mut key = String::new();
        until(&format!("{label} is another machine"), || {
            let row = self.row(label).filter(|row| row["state"] == "connected");
            key = row.map_or_else(String::new, |row| row["id"].as_str().unwrap().to_owned());
            !key.is_empty() && key != before
        });
        key
    }

    pub(super) fn cards(&self) -> Vec<Value> {
        self.requests.views(crate::hooks::now_ms())
    }

    /// Waits until exactly one card shows and returns it.
    pub(super) fn card(&self) -> Value {
        until("one card shows", || self.cards().len() == 1);
        self.cards().remove(0)
    }

    /// Waits until the one card that shows can be answered again.
    pub(super) fn reached(&self) -> Value {
        until("the card is in reach", || {
            self.cards().len() == 1 && !self.cards().iter().any(marked)
        });
        self.cards().remove(0)
    }

    /// When the request of `card` came and when its hold ends.
    pub(super) fn times(&self, card: &Value) -> (i64, i64) {
        let (id, session) = (card["id"].as_str().unwrap(), card["sessionId"].as_str());
        let came = self.requests.waiting_at(AGENT.id, session.unwrap());
        (came.unwrap(), self.requests.deadline(id).unwrap())
    }

    /// How often `register_native` and `settle_native` were called for
    /// the request `id`.
    pub(super) fn calls(&self, id: &str) -> (usize, usize) {
        let made = |call: &str| self.requests.probe_calls(call, id);
        (made("register_native"), made("settle_native"))
    }

    /// How often each alert sounded so far: needs-you, finished.
    pub(super) fn sounds(&mut self) -> (usize, usize) {
        let snapshot = self.snapshot();
        let items = snapshot["soundAlerts"]["items"].as_array().unwrap();
        let of = |kind: &str| items.iter().filter(|item| item["kind"] == kind).count();
        (of("needs-you"), of("finished"))
    }

    pub(super) fn log(&self) -> String {
        std::fs::read_to_string(self.data.join("hermes-gateway.log")).unwrap_or_default()
    }

    /// Every master `ssh` process whose socket is in the masters' folder.
    pub(super) fn masters(&self) -> Vec<String> {
        let listed = std::process::Command::new("/bin/ps")
            .args(["-axww", "-o", "pid=,args="])
            .output()
            .expect("ps");
        let marker = format!("ControlPath=\"{}", self.masters.display());
        let listed = String::from_utf8_lossy(&listed.stdout);
        // The masters alone: a command or a forward is a short `ssh` of
        // its own that names the same socket.
        let master = |line: &&str| line.contains(&marker) && line.contains("ControlMaster=yes");
        let ours = listed.lines().filter(master);
        ours.map(str::to_owned).collect()
    }

    /// How many logins the `sshd` of host `n` took.
    pub(super) fn logins(&self, n: usize) -> usize {
        let said = std::fs::read_to_string(self.hosts[n].lab.at("sshd.log"));
        said.unwrap_or_default()
            .matches("Accepted publickey")
            .count()
    }

    /// How many connections the `sshd` of host `n` saw at all.
    pub(super) fn knocks(&self, n: usize) -> usize {
        let said = std::fs::read_to_string(self.hosts[n].lab.at("sshd.log"));
        let said = said.unwrap_or_default();
        said.matches("Connection from").count() + said.matches("[preauth]").count()
    }
}

/// A test that fails says what the connection logged; and the machines
/// stop first, so every master is closed while its `sshd` runs.
impl Drop for Fleet {
    fn drop(&mut self) {
        self.machines.take();
        if std::thread::panicking() {
            eprintln!("the connection's log:\n{}", self.log());
        }
    }
}

/// The answer that picks `choice` of the one question on `card`.
pub(super) fn pick(card: &Value, choice: &str) -> Value {
    let key = card["questions"][0]["answerKey"].as_str().unwrap();
    json!({"kind": "answer", "answers": {key: {"selected": [choice]}}})
}
