//! A throwaway `sshd` on loopback for tests: its own host key, user keys,
//! `known_hosts` and client configuration, all in one temporary folder.
//!
//! "Far" is this same machine, reached through that `sshd`, signed in as
//! the user who runs the tests. A test can therefore look at what a far
//! command left behind. It also means a lab login is a login to the real
//! account, so what such a login would read of the user's is turned off:
//!
//! - The client side names the lab's own key, `known_hosts` and "no agent"
//!   for every host name, and no client reads a configuration file but
//!   the lab's (`config_text`).
//! - The server side accepts only the lab's `authorized_keys`, and never
//!   runs `~/.ssh/rc` or reads `~/.ssh/environment` (`PermitUserRC no`,
//!   `PermitUserEnvironment no`). Host-based sign-in and X11 are off, so
//!   `~/.rhosts`, `~/.shosts` and `~/.Xauthority` are never opened.
//! - `sshd` runs each command with the account's login shell, which would
//!   read the user's startup file (`~/.zshenv` for zsh, `config.fish` for
//!   fish). The lab's `sshd` sets `ZDOTDIR` and `XDG_CONFIG_HOME` to a
//!   folder of its own, and `BASH_ENV` and `ENV` to nothing, so the shell
//!   reads the lab's startup file there instead, which only sets a marker
//!   a test looks for.
//!
//! What a lab login still reads: the system's own files (the account
//! database, `/etc/zshenv` where one exists), and whatever the far command
//! itself opens.
use crate::ssh_link::{FarCommand, Folder, Settings, Target};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// The folder, in the lab, the login shell reads its startup file from.
pub(crate) const SHELL: &str = "shell";
/// What the lab's startup file sets, naming the shell that read it.
pub(crate) const MARKER: &str = "AGENT_HUD_LAB_STARTUP";

pub(crate) struct Lab {
    dir: tempfile::TempDir,
    pub(crate) port: u16,
    sshd: Child,
}

/// The agent a test started. Ends with the test.
pub(crate) struct Agent(Child, pub(crate) PathBuf);

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn quiet(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn free_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
    listener.local_addr().expect("its address").port()
}

/// A port on 127.0.0.1 that refuses a connection. It lies below the range
/// the system hands out, so no other test's lab is ever given it. (A port
/// that is bound and not listening would not do: macOS drops what is sent
/// to one, and a connection there times out.)
pub(crate) fn closed_port() -> u16 {
    let start = std::process::id() % 10_000;
    (0..500)
        .map(|step| 20_000 + ((start + step * 37) % 10_000) as u16)
        .find(|port| {
            let at = SocketAddr::from((Ipv4Addr::LOCALHOST, *port));
            TcpStream::connect_timeout(&at, Duration::from_secs(2))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
        })
        .expect("a port that refuses")
}

/// Runs this test program's lab tests under `filter` in a process of their
/// own, and fails if one of them does. They are marked `#[ignore]` so the
/// program's other tests never share a process with them: a lab test
/// starts hundreds of processes, and each start shares this process's open
/// files with the new process for a moment, a held file lock among them.
/// The instance lock's tests, which let a lock go and take it again at
/// once, then find it still held (measured: 14 runs in 15).
pub(crate) fn in_a_process_of_their_own(filter: &str) {
    let ran = Command::new(std::env::current_exe().expect("this test program"))
        .args([filter, "--ignored", "--test-threads", "6"])
        .stdin(Stdio::null())
        .output()
        .expect("the test program starts");
    let said = String::from_utf8_lossy(&ran.stdout);
    println!("{said}");
    assert!(
        ran.status.success(),
        "a lab test failed:\n{said}\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(!said.contains("running 0 tests"), "no test under {filter}");
}

pub(crate) fn until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Every process below `pid`.
fn descendants(pid: u32) -> Vec<u32> {
    let listed = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output();
    let children: Vec<u32> =
        String::from_utf8_lossy(&listed.map(|out| out.stdout).unwrap_or_default())
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
    let mut all = children.clone();
    for child in children {
        all.extend(descendants(child));
    }
    all
}

impl Lab {
    /// The lab, or None where this machine cannot run one. On macOS, where
    /// the link is verified, that is a failure.
    pub(crate) fn start() -> Option<Self> {
        Self::start_with(&[])
    }

    /// A lab whose logins also get `env` from its `sshd`, which sets them
    /// over its own defaults, `HOME` and `PATH` among them. `{lab}` in a
    /// value is the lab's folder. A later ticket's far side is this
    /// machine, so its lab gives a login a home folder of the lab's own.
    pub(crate) fn start_with(env: &[(&str, &str)]) -> Option<Self> {
        match Self::try_start(env) {
            Ok(lab) => Some(lab),
            Err(why) if cfg!(target_os = "macos") => panic!("the lab sshd did not start: {why}"),
            Err(why) => {
                eprintln!("skipped, no lab sshd here: {why}");
                None
            }
        }
    }

    fn try_start(env: &[(&str, &str)]) -> Result<Self, String> {
        // A socket path holds 104 bytes, so the lab needs a short folder.
        let base = Some(std::env::temp_dir())
            .filter(|base| base.as_os_str().len() <= 52)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = tempfile::Builder::new()
            .prefix("ahl")
            .tempdir_in(base)
            .map_err(|error| error.to_string())?;
        let at = |name: &str| dir.path().join(name);
        let text = |name: &str| at(name).display().to_string();
        for key in ["hostkey", "id", "agentkey", "strangerkey", "otherhost"] {
            if !quiet(
                "ssh-keygen",
                &["-q", "-t", "ed25519", "-N", "", "-f", &text(key)],
            ) {
                return Err("ssh-keygen failed".into());
            }
        }
        let public =
            |name: &str| fs::read_to_string(at(&format!("{name}.pub"))).unwrap_or_default();
        fs::write(at("authorized_keys"), public("id") + &public("agentkey"))
            .map_err(|error| error.to_string())?;
        let here = dir.path().display().to_string();
        let more: String = env
            .iter()
            .map(|(name, value)| format!(" {name}={}", value.replace("{lab}", &here)))
            .collect();
        // The login shell's startup files, in place of the user's own.
        fs::create_dir_all(at(SHELL).join("fish")).map_err(|error| error.to_string())?;
        fs::write(at(SHELL).join(".zshenv"), format!("export {MARKER}=zsh\n"))
            .map_err(|error| error.to_string())?;
        fs::write(
            at(SHELL).join("fish").join("config.fish"),
            format!("set -gx {MARKER} fish\n"),
        )
        .map_err(|error| error.to_string())?;
        // A port found free can be taken before `sshd` binds it, by another
        // test's lab. So `sshd` itself says when it listens, and one that
        // could not bind is started again on another port.
        let mut started = None;
        for _ in 0..5 {
            let port = free_port();
            fs::write(
                at("sshd_config"),
                format!(
                    "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\n\
                     PidFile none\nUsePAM no\nStrictModes no\nPasswordAuthentication no\n\
                     KbdInteractiveAuthentication no\nAllowTcpForwarding yes\n\
                     AllowAgentForwarding yes\nMaxStartups 100\n\
                     PermitUserRC no\nPermitUserEnvironment no\nHostbasedAuthentication no\n\
                     IgnoreRhosts yes\nX11Forwarding no\n\
                     SetEnv ZDOTDIR={shell} XDG_CONFIG_HOME={shell} BASH_ENV=/dev/null ENV=/dev/null{more}\n",
                    text("hostkey"),
                    text("authorized_keys"),
                    shell = text(SHELL),
                ),
            )
            .map_err(|error| error.to_string())?;
            let log = fs::File::create(at("sshd.log")).map_err(|error| error.to_string())?;
            let mut sshd = Command::new("/usr/sbin/sshd")
                .args(["-D", "-e", "-f", &text("sshd_config")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .map_err(|error| error.to_string())?;
            let listening = format!("Server listening on 127.0.0.1 port {port}.");
            let mut up = false;
            until(Duration::from_secs(15), || {
                up = fs::read_to_string(at("sshd.log")).is_ok_and(|said| said.contains(&listening));
                up || !matches!(sshd.try_wait(), Ok(None))
            });
            if up {
                started = Some((port, sshd));
                break;
            }
            let _ = sshd.kill();
            let _ = sshd.wait();
        }
        let Some((port, sshd)) = started else {
            let said = fs::read_to_string(at("sshd.log")).unwrap_or_default();
            return Err(format!("sshd is not listening: {said}"));
        };
        let lab = Self { dir, port, sshd };
        fs::write(lab.at("known_hosts"), lab.host_line("hostkey")).map_err(|e| e.to_string())?;
        fs::write(lab.at("config"), lab.config_text()).map_err(|e| e.to_string())?;
        Ok(lab)
    }

    pub(crate) fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A `known_hosts` line for the lab's `sshd` with the named key.
    pub(crate) fn host_line(&self, key: &str) -> String {
        let public = fs::read_to_string(self.at(&format!("{key}.pub"))).unwrap_or_default();
        let mut parts = public.split_whitespace();
        format!(
            "[127.0.0.1]:{} {} {}\n",
            self.port,
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default()
        )
    }

    /// The client configuration: two names for the lab's `sshd`, and for
    /// every host, whatever its name, the lab's own key and `known_hosts`
    /// and no agent. A test changes a line by replacing it in this text,
    /// and adds a host by appending a `Host` block.
    pub(crate) fn config_text(&self) -> String {
        format!(
            "Host lab other\n  HostName 127.0.0.1\n  Port {}\nHost *\n  UserKnownHostsFile \"{}\"\n  \
             GlobalKnownHostsFile /dev/null\n  IdentityFile \"{}\"\n  IdentitiesOnly yes\n  \
             IdentityAgent none\n",
            self.port,
            self.at("known_hosts").display(),
            self.at("id").display(),
        )
    }

    pub(crate) fn settings(&self) -> Settings {
        Settings {
            config: Some(self.at("config")),
            connect_timeout: Duration::from_secs(10),
            alive_interval: Duration::from_secs(1),
            alive_count: 2,
            ..Settings::system()
        }
    }

    /// Settings whose configuration file holds `text`.
    pub(crate) fn settings_with(&self, name: &str, text: &str) -> Settings {
        fs::write(self.at(name), text).expect("the lab's configuration");
        Settings {
            config: Some(self.at(name)),
            ..self.settings()
        }
    }

    pub(crate) fn target(&self) -> Target {
        Target::new("lab", None, None, None).expect("a plain host")
    }

    /// The control folder. Its name holds a space, as the app's own data
    /// folder does.
    pub(crate) fn folder(&self) -> Folder {
        Folder::open(&self.at("c s")).expect("the lab's control folder")
    }

    /// The lab's own agent, holding one key whose private half is then
    /// deleted, so only the agent can sign with it.
    pub(crate) fn agent(&self) -> Agent {
        let socket = self.at("agent.sock");
        let agent = Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("ssh-agent");
        let agent = Agent(agent, socket.clone());
        assert!(until(Duration::from_secs(5), || socket.exists()));
        let added = Command::new("ssh-add")
            .arg(self.at("agentkey"))
            .env("SSH_AUTH_SOCK", &socket)
            .stdin(Stdio::null())
            .output()
            .expect("ssh-add");
        assert!(added.status.success(), "{added:?}");
        fs::remove_file(self.at("agentkey")).expect("the private half goes");
        agent
    }

    /// Every `ssh` process that names a socket in `folder`.
    pub(crate) fn link_processes(&self, folder: &Folder) -> Vec<String> {
        let listed = Command::new("/bin/ps")
            .args(["-axww", "-o", "pid=,args="])
            .output()
            .expect("ps");
        // As the master is given it: a `%` in the path is doubled.
        let marker = format!("ControlPath=\"{}", folder.path().display()).replace('%', "%%");
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter(|line| line.contains(&marker))
            .map(str::to_owned)
            .collect()
    }

    /// Stops the far side of every open connection without closing it: the
    /// far machine went silent.
    pub(crate) fn freeze(&self) {
        for pid in descendants(self.sshd.id()) {
            // SAFETY: a signal to a process of the lab's own `sshd`.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for pid in descendants(self.sshd.id()) {
            // SAFETY: a signal to a process of the lab's own `sshd`.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
        let _ = self.sshd.kill();
        let _ = self.sshd.wait();
    }
}

/// The names in a folder, sorted.
pub(crate) fn entries(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Whether a process is there at all (signal 0 is sent nowhere).
pub(crate) fn runs(pid: u32) -> bool {
    // SAFETY: signal 0 only checks the pid.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

pub(crate) fn far<'a>(script: &'a str, args: &'a [&'a str]) -> FarCommand<'a> {
    FarCommand {
        script,
        args,
        stdin: None,
        limit: Duration::from_secs(20),
        most_output: 64 * 1024,
    }
}
