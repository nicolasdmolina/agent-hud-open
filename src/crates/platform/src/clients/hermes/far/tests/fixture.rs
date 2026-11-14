//! A far machine for the tests: the lab's `sshd`, a login whose home folder
//! is the lab's own, an open link, and pretend gateways on real sockets.
use super::super::{Far, RELAY};
use crate::ssh_link::{
    unix::tests::lab::{far, until, Lab},
    End, Folder, Link,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

/// A relay deadline short enough to wait out in a test.
pub(super) const SHORT: u32 = 3;

pub(super) struct FarLab {
    pub link: Link,
    #[allow(dead_code)]
    folder: Folder,
    /// The lab's folder, with its links resolved, as the far side sees it.
    pub dir: PathBuf,
    pub lab: Lab,
    sockets: AtomicUsize,
}

pub(super) fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("the mode");
}

/// A folder of the user's own that no one else may enter, as Hermes makes
/// its homes.
pub(super) fn private_folder(path: &Path) {
    fs::create_dir_all(path).expect("the folder");
    chmod(path, 0o700);
}

impl FarLab {
    /// A far login whose home folder is `<lab>/far`.
    pub(super) fn start() -> Option<Self> {
        Self::start_with(&[])
    }

    /// The same, with more of the login's environment set.
    pub(super) fn start_with(env: &[(&str, &str)]) -> Option<Self> {
        let mut all = vec![("HOME", "{lab}/far")];
        all.extend_from_slice(env);
        let lab = Lab::start_with(&all)?;
        let dir = fs::canonicalize(lab.at("")).expect("the lab's folder");
        fs::create_dir_all(dir.join("far")).expect("the far home folder");
        let folder = lab.folder();
        let link = Link::open(&folder, &lab.target(), &lab.settings()).expect("the link");
        // Before anything asks about Hermes: this login's home folder is
        // the lab's, so nothing below can look at the user's own.
        let ran = link
            .run(&far("printf %s \"$HOME\"", &[]))
            .expect("a command");
        assert_eq!(ran.end, End::Ended(0));
        let home = fs::canonicalize(String::from_utf8_lossy(&ran.stdout).as_ref());
        assert_eq!(home.ok(), Some(dir.join("far")), "the far home folder");
        Some(Self {
            link,
            folder,
            dir,
            lab,
            sockets: AtomicUsize::new(0),
        })
    }

    /// The default root home, `<lab>/far/.hermes`, made if it is not there.
    pub(super) fn root(&self) -> PathBuf {
        let root = self.dir.join("far").join(".hermes");
        private_folder(&root);
        root
    }

    /// The home of the profile `name` under the default root.
    pub(super) fn profile(&self, name: &str) -> PathBuf {
        let home = self.root().join("profiles").join(name);
        private_folder(&home);
        home
    }

    /// A listening socket at `path`, closed to everyone else. It is bound
    /// under a short name and moved, so a long home is no trouble.
    pub(super) fn listen(&self, path: &Path) -> UnixListener {
        let number = self.sockets.fetch_add(1, Ordering::Relaxed);
        let short = self.dir.join(format!("s{number}"));
        let listener = UnixListener::bind(&short).expect("a socket");
        chmod(&short, 0o600);
        fs::rename(&short, path).expect("the socket in its place");
        listener
    }

    /// A gateway whose socket is `<home>/gateway.sock`.
    pub(super) fn gate(&self, home: &Path, manner: Manner) -> Gate {
        Gate::serve(self.listen(&home.join("gateway.sock")), manner)
    }

    /// Where Hermes puts the socket of a home that cannot hold one: a
    /// folder named for the home, under a temporary root.
    pub(super) fn pointed(&self, home: &Path) -> PathBuf {
        let digest = Sha256::digest(home.to_string_lossy().as_bytes());
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        self.dir
            .join("t")
            .join(format!("hermes-gw-{}", &hex[..16]))
            .join("control.sock")
    }

    /// A gateway whose socket is where Hermes's pointer file would name
    /// it, with that pointer file in the home.
    pub(super) fn gate_by_pointer(&self, home: &Path, manner: Manner) -> Gate {
        let socket = self.pointed(home);
        private_folder(socket.parent().expect("its folder"));
        let gate = Gate::serve(self.listen(&socket), manner);
        write_pointer(home, socket.to_string_lossy().as_bytes());
        gate
    }

    /// How many relays run on the far side for `home`.
    pub(super) fn relays(&self, home: &Path) -> usize {
        let listed = Command::new("/bin/ps")
            .args(["-axww", "-o", "args="])
            .output()
            .expect("ps");
        let home = home.to_string_lossy();
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter(|line| line.contains(" -I -S -B - ") && line.contains(home.as_ref()))
            // Not the `ssh` client that carries the command.
            .filter(|line| !line.contains("ControlPath"))
            .count()
    }
}

pub(super) fn write_pointer(home: &Path, names: &[u8]) {
    let pointer = home.join("gateway.sock.path");
    fs::write(&pointer, names).expect("the pointer file");
    chmod(&pointer, 0o600);
}

pub(super) fn text(path: &Path) -> &str {
    path.to_str().expect("a plain path")
}

/// The relay with `more` put in front of its last line, which starts it:
/// how a test stands in for something the test machine cannot give, such
/// as a listener of another user. The app only ever sends the relay as it
/// is in the repository.
pub(super) fn far_with(seconds: u32, more: &str) -> Far {
    let ending = "\nmain()\n";
    assert!(RELAY.ends_with(ending), "the relay starts on its last line");
    let before = &RELAY[..RELAY.len() - ending.len()];
    Far::with_program(seconds, format!("{before}\n{more}{ending}"))
}

/// How a pretend gateway treats a connection.
#[derive(Clone)]
pub(super) enum Manner {
    /// Answers `identify` as the instance `instance` serving `served`,
    /// and grants a ticket holding `ticket`.
    Gateway {
        instance: String,
        served: Vec<PathBuf>,
        ticket: String,
    },
    /// Answers every line with this one.
    Says(Value),
    /// Takes the line and never answers.
    Silent,
    /// Takes the line and answers a byte now and then, with no end.
    Drips,
    /// Takes the line and answers more than the relay passes on.
    Floods,
    /// Closes without a word this many times, then answers as `Says`.
    EmptyThen(usize, Value),
}

impl Manner {
    pub(super) fn gateway(instance: &str, served: &[&Path]) -> Self {
        Self::Gateway {
            instance: instance.to_owned(),
            served: served.iter().map(|home| home.to_path_buf()).collect(),
            ticket: "a-ticket-of-the-lab".to_owned(),
        }
    }
}

/// A pretend gateway's control socket. It keeps every line it was sent.
pub(super) struct Gate {
    seen: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
}

impl Gate {
    pub(super) fn serve(listener: UnixListener, manner: Manner) -> Self {
        let gate = Self {
            seen: Arc::default(),
            connections: Arc::default(),
            running: Arc::new(AtomicBool::new(true)),
        };
        let (seen, connections, running) = (
            gate.seen.clone(),
            gate.connections.clone(),
            gate.running.clone(),
        );
        listener.set_nonblocking(true).expect("a listener");
        thread::spawn(move || {
            while running.load(Ordering::Acquire) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let number = connections.fetch_add(1, Ordering::AcqRel);
                let (seen, manner, running) = (seen.clone(), manner.clone(), running.clone());
                thread::spawn(move || answer(stream, number, &manner, &seen, &running));
            }
        });
        gate
    }

    /// Every line this gateway was sent.
    pub(super) fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// How many times it was connected to.
    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::Acquire)
    }

    /// Waits until it has been connected to `count` times.
    pub(super) fn connected(&self, count: usize) -> bool {
        until(Duration::from_secs(10), || self.connections() >= count)
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

fn answer(
    stream: UnixStream,
    number: usize,
    manner: &Manner,
    seen: &Mutex<Vec<String>>,
    running: &AtomicBool,
) {
    // Neither can be set once the other end has gone, which a relay that
    // refuses is at once; the read below then ends by itself.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).unwrap_or(0) == 0 {
        // Connected to and told nothing.
        return;
    }
    seen.lock().unwrap().push(line.trim().to_owned());
    let asked: Value = serde_json::from_str(&line).unwrap_or_default();
    let mut out = &stream;
    let says = |value: &Value| format!("{value}\n").into_bytes();
    match manner {
        Manner::Gateway {
            instance,
            served,
            ticket,
        } => {
            let _ = out.write_all(&says(&gateway(&asked, instance, served, ticket)));
        }
        Manner::Says(value) => {
            let _ = out.write_all(&says(value));
        }
        Manner::EmptyThen(times, value) if number >= *times => {
            let _ = out.write_all(&says(value));
        }
        Manner::EmptyThen(..) => {}
        Manner::Silent => {
            // Held open until the other end goes or the test is over.
            let mut rest = [0u8; 64];
            while running.load(Ordering::Acquire) && !matches!((&stream).read(&mut rest), Ok(0)) {}
        }
        Manner::Drips => {
            while running.load(Ordering::Acquire) && out.write_all(b"x").is_ok() {
                thread::sleep(Duration::from_millis(100));
            }
        }
        Manner::Floods => {
            let chunk = [b'x'; 64 * 1024];
            for _ in 0..12 {
                if out.write_all(&chunk).is_err() {
                    break;
                }
            }
            let _ = out.write_all(b"\n");
        }
    }
}

/// What a real gateway answers, from the recording of Hermes 0.22.5.
fn gateway(asked: &Value, instance: &str, served: &[PathBuf], ticket: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agents/tests/fixtures/hermes/gateway/gateway-self-description.json");
    let said: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("the recording")).unwrap();
    let said = &said["control_socket"];
    match asked["verb"].as_str() {
        Some("identify") => {
            let mut identify = said["identify_throwaway_ready_two_profiles"].clone();
            let homes: Vec<Value> = served
                .iter()
                .map(|home| json!({"profile_id": home, "home": home}))
                .collect();
            identify["result"]["hermes_home"] = json!(served.first());
            identify["result"]["instance_id"] = json!(instance);
            identify["result"]["served_profiles"] = json!(homes);
            identify
        }
        Some("session-ticket") if asked["params"]["instance_id"] != instance => {
            said["refused_stale_instance"].clone()
        }
        Some("session-ticket") => {
            let mut grant = said["session_ticket_grant"].clone();
            grant["ticket"] = json!(ticket);
            grant["instance_id"] = json!(instance);
            grant["profile_id"] = asked["params"]["profile_id"].clone();
            json!({"ok": true, "protocol": 1, "id": 1, "result": grant})
        }
        _ => said["unknown_verb"].clone(),
    }
}
