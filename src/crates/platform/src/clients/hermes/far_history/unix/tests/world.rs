//! One lab with a far Hermes home in it, and the far side's own view of
//! the readers that run there.
use super::super::{Reader, Settings};
use super::schema;
use crate::ssh_link::unix::tests::lab::{Lab, BIN};
use crate::ssh_link::{FarCommand, Folder, Link, Ran};
use agent_hud_agents::hermes::far_history::{self as far, Cell, SESSION_COLUMNS, USAGE_COLUMNS};
use agent_hud_agents::hermes::gateway::{machine_key, MachineKey};
use rusqlite::{types::ValueRef, Connection};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The system's Python, which a stand-in `python3` hands over to.
pub(super) const PYTHON: &str = "/usr/bin/python3";

pub(super) struct World {
    pub(super) lab: Lab,
    pub(super) folder: Folder,
    /// The far home, as its own canonical path.
    pub(super) home: String,
    /// Every line the reader logged, with when.
    pub(super) log: Arc<Mutex<Vec<(Instant, String)>>>,
    /// The far side's Hermes: a gateway that runs has the database open,
    /// so its write-ahead log and shared memory files are there. Let go,
    /// the database is one nobody has open (`World::stop_hermes`).
    hermes: Mutex<Option<Connection>>,
}

/// Times short enough for a test: a reader lives two seconds.
pub(super) fn settings() -> Settings {
    Settings {
        deadline: Duration::from_secs(2),
        margin: Duration::from_secs(2),
        local_wait: Duration::from_secs(15),
        ..Settings::default()
    }
}

/// The key of the machine whose root home is `home`. Every lab has its
/// own, as every far machine does, so one lab's readers never count as
/// another's.
pub(super) fn machine_of(home: &str) -> MachineKey {
    machine_key(Some("0123456789abcdef0123456789abcdef"), home).expect("a key")
}

impl World {
    pub(super) fn start() -> Option<Self> {
        Self::on(Lab::start()?)
    }

    pub(super) fn on(lab: Lab) -> Option<Self> {
        fs::create_dir_all(lab.at("home")).expect("the far home");
        // The lab is under a folder the system reaches through a link.
        let home = fs::canonicalize(lab.at("home")).expect("its own path");
        let folder = lab.folder();
        let world = Self {
            lab,
            folder,
            home: home.display().to_string(),
            log: Arc::default(),
            hermes: Mutex::new(None),
        };
        let db = world.db();
        let mode: String = db
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .expect("the journal mode");
        assert_eq!(mode, "wal");
        db.execute_batch(schema::HERMES).expect("Hermes's schema");
        *world.hermes.lock().unwrap() = Some(db);
        Some(world)
    }

    pub(super) fn link(&self) -> Link {
        Link::open(&self.folder, &self.lab.target(), &self.lab.settings()).expect("a link")
    }

    /// A reader whose log lines are kept, and printed for `--nocapture`.
    pub(super) fn reader(&self, settings: Settings) -> Reader {
        let log = Arc::clone(&self.log);
        Reader::new(settings, move |line| {
            println!("  log: {line}");
            log.lock().unwrap().push((Instant::now(), line.to_owned()));
        })
    }

    pub(super) fn logged(&self) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter().map(|(_, line)| line.clone()).collect()
    }

    pub(super) fn database(&self) -> PathBuf {
        PathBuf::from(&self.home).join("state.db")
    }

    /// The far side's own connection to the database: the writer Hermes
    /// would be.
    pub(super) fn db(&self) -> Connection {
        Connection::open(self.database()).expect("the far database")
    }

    /// Hermes stops: its last connection closes, which folds the
    /// write-ahead log into the database and removes it.
    pub(super) fn stop_hermes(&self) {
        *self.hermes.lock().unwrap() = None;
    }

    pub(super) fn machine(&self) -> MachineKey {
        machine_of(&self.home)
    }

    pub(super) fn mark(&self) -> String {
        far::mark(&self.machine(), &self.home)
    }

    /// Puts a stand-in for `python3` first on the far path.
    pub(super) fn stand_in(&self, script: &str) {
        let path = self.lab.at(BIN).join("python3");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("the stand-in");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("its mode");
    }

    pub(super) fn no_stand_in(&self) {
        let _ = fs::remove_file(self.lab.at(BIN).join("python3"));
    }

    /// The names in the far home, sorted.
    pub(super) fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.home)
            .expect("the far home")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// Replaces the `sessions` table with one whose every read never
    /// ends: a query made slow on purpose, which holds the database open
    /// until something ends the reader.
    pub(super) fn make_every_read_endless(&self) {
        self.db()
            .execute_batch(
                "DROP TABLE sessions;
                 CREATE VIEW sessions AS WITH RECURSIVE spin(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM spin)
                 SELECT n AS rowid, 'endless' AS id, 1.0 AS started_at FROM spin WHERE n < 0;",
            )
            .expect("the endless view");
    }

    /// A session with `usage` usage rows, each with a task and a base URL
    /// of its own.
    pub(super) fn add_session(&self, db: &Connection, id: &str, usage: usize) {
        let number = db
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("a count");
        let ended = (number % 2 == 0).then_some(1_760_000_900.25 + number as f64);
        db.execute(
            "INSERT INTO sessions (id, source, model, started_at, ended_at, input_tokens, output_tokens, cache_read_tokens,
                cache_write_tokens, cwd, billing_provider, billing_base_url, billing_mode, title, last_activity_at, profile_name)
             VALUES (?1, 'telegram', 'claude-fable-5', ?2, ?3, ?4, 20, 300, 4, '/Users/far/work', 'anthropic',
                'https://api.anthropic.com', 'api', ?5, ?6, NULL)",
            (
                id,
                1_760_000_000.5 + number as f64,
                ended,
                1_000 + number,
                format!("Made-up title {number} with a \"quote\" and an é"),
                1_760_000_600.75 + number as f64,
            ),
        )
        .expect("a session");
        for row in 0..usage {
            let task = if row == 0 {
                String::new()
            } else {
                format!("aux:title-{row}")
            };
            db.execute(
                "INSERT INTO session_model_usage (session_id, model, billing_provider, billing_base_url, billing_mode, task,
                    api_call_count, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, first_seen, last_seen)
                 VALUES (?1, ?2, 'openrouter', ?3, 'api', ?4, 1, ?5, 7, 11, 13, ?6, ?7)",
                (
                    id,
                    format!("model-{}", row % 3),
                    format!("https://openrouter.ai/api/v{}", row % 2 + 1),
                    task,
                    100 + row as i64,
                    1_760_000_010.125 + row as f64,
                    1_760_000_500.5 + row as f64,
                ),
            )
            .expect("a usage row");
        }
    }
}

fn cell(value: ValueRef<'_>) -> Cell {
    match value {
        ValueRef::Null | ValueRef::Blob(_) => Cell::Null,
        ValueRef::Integer(whole) => Cell::Int(whole),
        ValueRef::Real(number) => Cell::Real(number),
        ValueRef::Text(text) => Cell::Text(String::from_utf8_lossy(text).into_owned()),
    }
}

/// A session's row and its usage rows, read here straight from the
/// database, in the reader's column order. The usage rows are sorted, as
/// a caller sorts the reader's before comparing.
pub(super) fn direct(db: &Connection, id: &str) -> (Vec<Cell>, Vec<Vec<Cell>>) {
    let session = format!(
        "SELECT {} FROM sessions WHERE id=?1",
        SESSION_COLUMNS.join(",")
    );
    let session = db
        .query_row(&session, [id], |row| {
            Ok((0..SESSION_COLUMNS.len())
                .map(|at| cell(row.get_ref_unwrap(at)))
                .collect())
        })
        .expect("the session");
    let usage = format!(
        "SELECT {} FROM session_model_usage WHERE session_id=?1",
        USAGE_COLUMNS.join(",")
    );
    let mut usage = db.prepare(&usage).expect("the usage statement");
    let rows = usage
        .query_map([id], |row| {
            Ok((0..USAGE_COLUMNS.len())
                .map(|at| cell(row.get_ref_unwrap(at)))
                .collect())
        })
        .expect("the usage rows");
    let mut usage: Vec<Vec<Cell>> = rows.map(|row| row.expect("a row")).collect();
    sort(&mut usage);
    (session, usage)
}

pub(super) fn sort(rows: &mut [Vec<Cell>]) {
    rows.sort_by_key(|row| format!("{row:?}"));
}

fn listed() -> Vec<(u32, String)> {
    let listed = Command::new("/bin/ps")
        .args(["-A", "-ww", "-o", "pid=,args="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, args) = line.trim_start().split_once(' ')?;
            Some((pid.parse().ok()?, args.to_owned()))
        })
        .collect()
}

/// The far reader processes of a home, as the far process list shows
/// them: every process that carries the home's mark as one argument.
pub(super) fn readers(mark: &str) -> Vec<u32> {
    let mut found: Vec<u32> = listed()
        .into_iter()
        .filter(|(_, args)| args.split_whitespace().any(|word| word == mark))
        .map(|(pid, _)| pid)
        .collect();
    found.sort_unstable();
    found
}

/// The processes whose arguments hold `text`.
pub(super) fn naming(text: &str) -> Vec<u32> {
    let found = listed().into_iter().filter(|(_, args)| args.contains(text));
    found.map(|(pid, _)| pid).collect()
}

/// The local `ssh` clients that carry a read of that home to the far
/// side. Their one argument holds the mark inside quotes, so the far
/// reader's own listing never takes one for a reader.
pub(super) fn clients(mark: &str) -> Vec<u32> {
    let quoted = format!("'{mark}'");
    listed()
        .into_iter()
        .filter(|(_, args)| args.contains(&quoted) && args.contains("link.invalid"))
        .map(|(pid, _)| pid)
        .collect()
}

/// Which of `pids` has the database open, by the system's list of open
/// files.
pub(super) fn open(pids: &[u32], database: &str) -> Vec<u32> {
    if pids.is_empty() {
        return Vec::new();
    }
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    let listed = Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pids.join(","), "-Fpn"])
        .output()
        .expect("lsof");
    let mut found = Vec::new();
    let mut pid = 0;
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        if let Some(number) = line.strip_prefix('p') {
            pid = number.parse().unwrap_or(0);
        } else if line.strip_prefix('n') == Some(database) && !found.contains(&pid) {
            found.push(pid);
        }
    }
    found
}

/// One look at the far side.
#[derive(Clone, Debug)]
pub(super) struct Sample {
    pub(super) at: Instant,
    pub(super) readers: Vec<u32>,
    /// The readers that had the database open.
    pub(super) open: Vec<u32>,
}

/// Looks at the far side again and again until it is stopped.
pub(super) struct Watch {
    stop: Arc<AtomicBool>,
    looks: Option<JoinHandle<Vec<Sample>>>,
}

impl Watch {
    pub(super) fn start(world: &World) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (mark, database) = (world.mark(), world.database().display().to_string());
        let stopped = Arc::clone(&stop);
        let looks = thread::spawn(move || {
            let mut samples = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let readers = readers(&mark);
                let open = open(&readers, &database);
                samples.push(Sample {
                    at: Instant::now(),
                    readers,
                    open,
                });
                thread::sleep(Duration::from_millis(15));
            }
            samples
        });
        Self {
            stop,
            looks: Some(looks),
        }
    }

    pub(super) fn finish(mut self) -> Vec<Sample> {
        self.stop.store(true, Ordering::SeqCst);
        self.looks
            .take()
            .expect("the looks")
            .join()
            .expect("the watch")
    }
}

/// What a watch saw, in a line, and its two highest counts.
pub(super) fn most(samples: &[Sample]) -> (usize, usize) {
    let readers = samples
        .iter()
        .map(|sample| sample.readers.len())
        .max()
        .unwrap_or(0);
    let open = samples
        .iter()
        .map(|sample| sample.open.len())
        .max()
        .unwrap_or(0);
    let mut pids: Vec<u32> = samples
        .iter()
        .flat_map(|sample| sample.open.clone())
        .collect();
    pids.sort_unstable();
    pids.dedup();
    println!(
        "  far side: {} looks, at most {readers} reader process(es), at most {open} with the database open; had it open: {pids:?}",
        samples.len()
    );
    (readers, open)
}

/// Runs the program once, with no caller and no slot: what the far side
/// does by itself.
pub(super) fn raw(link: &Link, home: &str, deadline: Duration, request: &far::Request) -> Ran {
    let args = far::arguments(&machine_of(home), home, deadline, far::MOST_OUTPUT, request);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    link.run(&FarCommand {
        script: far::SCRIPT,
        args: &args,
        stdin: Some(far::PROGRAM.as_bytes()),
        limit: Duration::from_secs(40),
        most_output: 4 << 20,
    })
    .expect("the link ran it")
}

pub(super) fn sessions(ids: &[&str]) -> far::Request {
    far::Request::Sessions {
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

pub(super) fn kill(pid: u32) {
    // SAFETY: a signal to a process of this test's own lab.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}
