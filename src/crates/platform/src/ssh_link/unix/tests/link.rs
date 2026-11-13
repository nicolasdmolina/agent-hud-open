//! Open, run, forward, cancel, close; and what no configuration file can
//! change.
use super::{
    super::{
        args::{self, Ask},
        PROGRAM,
    },
    lab::{entries, far, free_port, runs, until, Lab},
    pure::DESIGN,
};
use crate::ssh_link::{End, Failure, Folder, Link, Lost, Refused, Target};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    process::{Command, Stdio},
    time::Duration,
};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// Sends `ping` through `local` and answers `pong` from the far listener.
fn exchange(far: &TcpListener, local: u16) {
    far.set_nonblocking(true).unwrap();
    let mut near = TcpStream::connect(loopback(local)).expect("the forward listens");
    near.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    near.write_all(b"ping").unwrap();
    let mut reached = None;
    assert!(
        until(Duration::from_secs(10), || {
            reached = far.accept().ok();
            reached.is_some()
        }),
        "nothing reached the far listener"
    );
    let (mut there, _) = reached.unwrap();
    there.set_nonblocking(false).unwrap();
    there
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut word = [0u8; 4];
    there.read_exact(&mut word).unwrap();
    assert_eq!(&word, b"ping");
    there.write_all(b"pong").unwrap();
    near.read_exact(&mut word).unwrap();
    assert_eq!(&word, b"pong");
}

#[test]
fn open_run_forward_exchange_cancel_close_leaves_nothing() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).expect("the link opens");
    let pid = link.master_pid().unwrap();
    assert_eq!(link.alive(), Ok(()));

    let ran = link
        .run(&far(
            "printf '%s' \"$1\"; echo oops >&2; exit 7",
            &["hello"],
        ))
        .unwrap();
    assert_eq!(ran.end, End::Ended(7));
    assert_eq!(ran.stdout, b"hello");
    assert_eq!(ran.stderr, "oops\n");
    // 255 is a far program's status too, told from a lost link.
    assert_eq!(
        link.run(&far("exit 255", &[])).unwrap().end,
        End::Ended(255)
    );

    let listener = TcpListener::bind(loopback(0)).unwrap();
    let forward = link.forward(listener.local_addr().unwrap()).unwrap();
    assert_ne!(forward.local, forward.far);
    exchange(&listener, forward.local);
    link.cancel(forward).unwrap();
    assert!(TcpStream::connect(loopback(forward.local)).is_err());
    assert_eq!(
        link.cancel(forward),
        Err(Failure::Refused(Refused::NotForwarded))
    );

    // A forward still open at close is cancelled by it.
    let held = link.forward(listener.local_addr().unwrap()).unwrap();
    let closed = link.close();
    println!("closed: {closed:?}");
    assert!(closed.gone);
    assert_eq!(closed.cancelled, 1);
    assert!(closed.exit.is_some(), "the master's exit code is recorded");
    assert!(TcpStream::connect(loopback(held.local)).is_err());
    assert!(!runs(pid), "the master's process is gone");
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
    assert_eq!(
        link.run(&far("true", &[])).unwrap_err(),
        Failure::Refused(Refused::Closed)
    );
    assert_eq!(link.close(), closed);
}

#[test]
fn the_running_masters_arguments_hold_every_option() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = crate::ssh_link::Settings {
        config: lab.settings().config,
        ..crate::ssh_link::Settings::system()
    };
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    let pid = link.master_pid().unwrap().to_string();
    let listed = Command::new("/bin/ps")
        .args(["-ww", "-o", "args=", "-p", &pid])
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout).trim().to_owned();
    println!("master argument list: {listed}");
    for option in DESIGN {
        assert!(
            listed.contains(&format!("-o {option} ")),
            "{option}: {listed}"
        );
    }
    let own = format!("-o ControlPath=\"{}/", folder.path().display());
    assert!(listed.contains(&own), "{listed}");
    assert!(listed.ends_with(" -- lab"), "{listed}");
    link.close();
}

#[test]
fn a_configuration_file_cannot_weaken_the_link() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let agent = lab.agent();
    let configured = free_port();
    // What an unsafe file would say, with an agent there to be forwarded.
    let hostile = format!(
        "{}  StrictHostKeyChecking no\n  ForwardAgent yes\n  LocalForward 127.0.0.1:{configured} 127.0.0.1:{}\n",
        lab.config_text().replace(
            "IdentityAgent none",
            &format!("IdentityAgent \"{}\"", agent.1.display())
        ),
        lab.port,
    );

    // An unknown host is still refused, and nothing is written.
    std::fs::write(lab.at("empty"), "").unwrap();
    let unknown = hostile.replace("known_hosts", "empty");
    let refused = Link::open(
        &folder,
        &lab.target(),
        &lab.settings_with("unknown", &unknown),
    );
    assert_eq!(refused.err(), Some(Failure::Lost(Lost::NotTrusted)));
    assert_eq!(std::fs::read(lab.at("empty")).unwrap(), b"");

    let settings = lab.settings_with("hostile", &hostile);
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    let seen = link
        .run(&far("printf '%s' \"${SSH_AUTH_SOCK-none}\"", &[]))
        .unwrap();
    assert_eq!(seen.stdout, b"none", "no agent socket on the far side");
    assert!(
        TcpStream::connect(loopback(configured)).is_err(),
        "the configured forward is not open"
    );
    link.close();

    // The same file without the link's options does both, so the two
    // checks above could have failed.
    let mut plain = Command::new("/usr/bin/ssh")
        .args(["-F", &lab.at("hostile").display().to_string()])
        .args(["-o", "BatchMode=yes", "lab"])
        .arg("printf '%s' \"${SSH_AUTH_SOCK-none}\"; sleep 5")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(
        until(Duration::from_secs(4), || {
            TcpStream::connect(loopback(configured)).is_ok()
        }),
        "a plain ssh opens the configured forward"
    );
    let mut said = String::new();
    plain
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut said)
        .unwrap();
    plain.wait().unwrap();
    assert!(
        said.starts_with('/'),
        "a plain ssh forwards the agent: {said}"
    );
}

#[test]
fn a_forward_listens_on_loopback_only_and_leads_only_to_the_far_loopback() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    for far in [
        "10.1.2.3:80",
        "127.0.0.2:80",
        "0.0.0.0:80",
        "[::1]:80",
        "127.0.0.1:0",
    ] {
        assert_eq!(
            link.forward(far.parse().unwrap()),
            Err(Failure::Refused(Refused::FarAddress)),
            "{far}"
        );
    }
    let listener = TcpListener::bind(loopback(0)).unwrap();
    let forward = link.forward(listener.local_addr().unwrap()).unwrap();
    exchange(&listener, forward.local);
    // The master listens for this forward, on 127.0.0.1 and nowhere else.
    let pid = link.master_pid().unwrap().to_string();
    let listening = Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN"])
        .output()
        .unwrap();
    let listening = String::from_utf8_lossy(&listening.stdout);
    println!("{listening}");
    let sockets: Vec<&str> = listening.lines().filter(|l| l.contains("LISTEN")).collect();
    assert_eq!(sockets.len(), 1, "{listening}");
    assert!(sockets[0].contains(&format!("TCP 127.0.0.1:{} ", forward.local)));
    assert!(TcpStream::connect(("::1", forward.local)).is_err());
    link.close();
}

#[test]
fn a_host_that_is_not_plain_makes_no_target_and_starts_no_ssh() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let marker = lab.at("proxied");
    let hosts = [
        format!("-oProxyCommand=touch {}", marker.display()),
        "-oProxyCommand=true".to_owned(),
        "a b".to_owned(),
        "a;b".to_owned(),
    ];
    for host in &hosts {
        // No target, so nothing to open a link with: `Link::open` takes
        // a `Target` and there is no other way to make one.
        assert_eq!(Target::new(host, None, None, None), Err(Refused::Address));
        assert_eq!(
            Target::new("lab", Some(host), None, None),
            Err(Refused::Address)
        );
    }
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
    assert!(!marker.exists());
}

#[test]
fn a_far_path_with_a_quote_a_space_and_a_dollar_arrives_as_plain_text() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let path = lab.at("it's a $HOME `id` \"q\" \\ *.txt");
    std::fs::write(&path, "inside").unwrap();
    let path = path.display().to_string();
    let ran = link
        .run(&far("printf '%s|' \"$1\"; cat \"$1\"", &[&path]))
        .unwrap();
    assert_eq!(ran.end, End::Ended(0), "{}", ran.stderr);
    assert_eq!(
        String::from_utf8(ran.stdout).unwrap(),
        format!("{path}|inside")
    );
    link.close();
}

/// With no master, a session's `ssh` would open a connection of its own.
/// A client is given a proxy that fails and a host no network resolves, so
/// it ends at once instead.
#[test]
fn a_client_with_no_master_does_not_dial() {
    let temp = tempfile::tempdir().unwrap();
    let folder = Folder::open(&temp.path().join("c")).unwrap();
    let nobody = folder.control("gone");
    let session = Command::new(PROGRAM)
        .args(args::client(&nobody, Ask::Session("echo reached")))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(session.status.code(), Some(255));
    assert_eq!(session.stdout, b"");
    let check = Command::new(PROGRAM)
        .args(args::client(&nobody, Ask::Check))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(255));
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}
