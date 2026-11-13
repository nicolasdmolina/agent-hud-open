//! The gateway's control socket: one JSON line in, one line out, then the
//! gateway closes it. It answers `identify` and grants the ticket that
//! opens one WebSocket. It is asked as the same user, and only where the
//! path to it is the user's own (`unix.rs`).
use serde_json::Value;
use std::{path::Path, time::Duration};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as host;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as host;

/// Why the control socket gave no answer. On Windows, where it is not
/// asked at all, nothing makes the refusals of a socket file.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Silent {
    /// No socket, or nothing listening on it: no gateway runs.
    Absent,
    /// It did not answer in time. Worth one more try before giving up.
    TimedOut,
    /// The home, the pointer, the socket or its listener is not the user's
    /// own, named by which.
    Unsafe(&'static str),
    /// What it answered is not one JSON line.
    Garbled,
}

/// Asks the gateway whose home is `home` one question. `home` has its links
/// resolved already.
pub(super) fn ask(home: &Path, line: &Value, timeout: Duration) -> Result<Value, Silent> {
    let mut request = line.to_string().into_bytes();
    request.push(b'\n');
    let answer = host::exchange(home, &request, timeout)?;
    let line = answer.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
    serde_json::from_slice(line).map_err(|_| Silent::Garbled)
}

/// The folder Hermes makes for the socket of `home` where the home cannot
/// hold one.
#[cfg(all(test, unix))]
pub(super) use unix::folder_of;
