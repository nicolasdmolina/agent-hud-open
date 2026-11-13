//! Every argument list `ssh` is started with. Two kinds: the master, which
//! dials, and a client, which only speaks to a master's control socket.
//!
//! Options on the command line win over every configuration file, so each
//! setting that could weaken the connection is given here, on every `ssh`
//! the link starts.
use super::super::{Forward, Settings, Target};
use std::ffi::OsString;

/// The design's list (spec section 6), less the three that differ between
/// master and client and the timing ones, which `master` adds.
pub(super) const FIXED: &[&str] = &[
    "BatchMode=yes",
    "StrictHostKeyChecking=yes",
    "UpdateHostKeys=no",
    "ForwardAgent=no",
    "ForwardX11=no",
    "PermitLocalCommand=no",
    "RequestTTY=no",
    "ControlPersist=no",
    "ExitOnForwardFailure=yes",
    "TCPKeepAlive=yes",
    // Beyond the design's list, each for the same reason: a configuration
    // file could otherwise send the master to the background where this
    // process no longer owns it, trust a key from DNS, skip the key check
    // for a loopback host, run a command or open a tunnel device, bind a
    // forward to every address, or put a key into the agent.
    "ForkAfterAuthentication=no",
    "VerifyHostKeyDNS=no",
    "NoHostAuthenticationForLocalhost=no",
    "RemoteCommand=none",
    "Tunnel=no",
    "GatewayPorts=no",
    "AddKeysToAgent=no",
    // The master's last words say why it ended ("Timeout, server … not
    // responding"), and a quieter level would drop them.
    "LogLevel=INFO",
];

/// A client's destination. It speaks to a control socket and never dials,
/// so it is given a name no network resolves, and a proxy that fails, in
/// case `ssh` ever tried: with no master, a session would otherwise open a
/// connection of its own.
pub(super) const NOWHERE: &str = "link.invalid";
const NO_DIAL: &str = "ProxyCommand=/usr/bin/false";

/// What a client asks of the master.
#[derive(Clone, Copy)]
pub(super) enum Ask<'a> {
    Check,
    Exit,
    Forward(Forward),
    Cancel(Forward),
    /// One command line for the far shell.
    Session(&'a str),
}

fn option(args: &mut Vec<OsString>, value: impl Into<OsString>) {
    args.push("-o".into());
    args.push(value.into());
}

/// The master: the one `ssh` that dials. It reads the user's configuration
/// (aliases, proxy commands), runs no command and holds no forward of its
/// own. `control` is the folder's spelling of the socket path.
pub(super) fn master(settings: &Settings, target: &Target, control: &str) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(config) = &settings.config {
        args.push("-F".into());
        args.push(config.into());
    }
    args.push("-N".into());
    option(&mut args, "ControlMaster=yes");
    option(&mut args, format!("ControlPath={control}"));
    option(&mut args, "ClearAllForwardings=yes");
    for fixed in FIXED {
        option(&mut args, *fixed);
    }
    let seconds = |time: std::time::Duration| time.as_secs().max(1);
    option(
        &mut args,
        format!("ServerAliveInterval={}", seconds(settings.alive_interval)),
    );
    option(
        &mut args,
        format!("ServerAliveCountMax={}", settings.alive_count.max(1)),
    );
    option(
        &mut args,
        format!("ConnectTimeout={}", seconds(settings.connect_timeout)),
    );
    if let Some(port) = target.port() {
        args.push("-p".into());
        args.push(port.to_string().into());
    }
    if let Some(key) = target.key() {
        // The path as written: `ssh` would read a `%` in it as a token.
        args.push("-i".into());
        args.push(key.to_string_lossy().replace('%', "%%").into());
    }
    args.push("--".into());
    args.push(target.destination().into());
    args
}

/// A client of the master at `control`. It reads no configuration file, so
/// a forward a file names for some host is never added beside ours.
pub(super) fn client(control: &str, ask: Ask) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-F".into(), "none".into()];
    option(&mut args, "ControlMaster=no");
    option(&mut args, format!("ControlPath={control}"));
    option(&mut args, NO_DIAL);
    for fixed in FIXED {
        option(&mut args, *fixed);
    }
    // `ssh` drops the very forward a request names when told to clear
    // forwards, and reports success. Every other request clears.
    let (operation, forward) = match ask {
        Ask::Check => (Some("check"), None),
        Ask::Exit => (Some("exit"), None),
        Ask::Forward(forward) => (Some("forward"), Some(forward)),
        Ask::Cancel(forward) => (Some("cancel"), Some(forward)),
        Ask::Session(_) => (None, None),
    };
    if forward.is_none() {
        option(&mut args, "ClearAllForwardings=yes");
    }
    if let Some(operation) = operation {
        args.push("-O".into());
        args.push(operation.into());
    }
    if let Some(forward) = forward {
        args.push("-L".into());
        args.push(spec(forward).into());
    }
    args.push("--".into());
    args.push(NOWHERE.into());
    if let Ask::Session(line) = ask {
        args.push(line.into());
    }
    args
}

/// Both ends are 127.0.0.1, written out: a bare port would listen on
/// whatever `GatewayPorts` allows.
pub(super) fn spec(forward: Forward) -> String {
    format!("127.0.0.1:{}:127.0.0.1:{}", forward.local, forward.far)
}
