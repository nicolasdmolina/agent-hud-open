//! The line sent to the far shell, and what `ssh`'s last words mean.
use super::super::{quote, Lost, Refused};

/// The longest command line sent. Over a master on macOS a request near
/// 8 KB fails outright, so a long program goes on standard input instead.
pub(super) const LONGEST_LINE: usize = 4096;

/// `script` under `sh -c`, with `args` as its `$1`, `$2`…. The login shell
/// there may be zsh or fish, whose rules differ; it only ever sees quoted
/// words and hands them to `sh`. No here-document is sent: the far shell
/// would write a temporary file for it, on a disk that may be full.
pub(super) fn far_line(script: &str, args: &[&str]) -> Result<String, Refused> {
    if script.contains("<<") {
        return Err(Refused::HereDocument);
    }
    let mut line = format!("exec sh -c {} sh", quote(script));
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    if line.contains('\0') {
        // No argument can hold one, so `ssh` would not start.
        return Err(Refused::Unavailable);
    }
    if line.len() > LONGEST_LINE {
        return Err(Refused::CommandTooLong);
    }
    Ok(line)
}

/// What the master said of this in passing while it ran well: a forward
/// that found no listener, a local port already taken.
fn passing(line: &str) -> bool {
    ["channel ", "channel_", "bind ", "mux_", "muxserver_"]
        .iter()
        .any(|start| line.starts_with(start))
}

/// Why a master ended, from the end of what it wrote. `ssh` words these in
/// English on every system.
pub(super) fn why_lost(log: &str) -> Lost {
    let last: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !passing(line))
        .rev()
        .take(3)
        .collect();
    let said = |words: &[&str]| {
        last.iter()
            .any(|line| words.iter().any(|word| line.contains(word)))
    };
    if said(&["has changed and you have requested strict checking"]) {
        Lost::KeyChanged
    } else if said(&["host key is known for", "Host key verification failed"]) {
        Lost::NotTrusted
    } else if said(&["Permission denied", "Too many authentication failures"]) {
        Lost::SignInRefused
    } else if said(&["timed out", "Timeout, server", "not responding"]) {
        Lost::TimedOut
    } else if said(&[
        "Could not resolve hostname",
        "Connection refused",
        "No route to host",
        "Network is unreachable",
        "Host is down",
        "Connection closed",
        "Connection reset",
        "Broken pipe",
    ]) {
        Lost::CannotReach
    } else {
        Lost::Ended
    }
}
