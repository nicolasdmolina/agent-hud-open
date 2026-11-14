//! The gateway's control socket: one JSON line in, one line out, then the
//! gateway closes it. It answers `identify` and grants the ticket that
//! opens one WebSocket. The exchange itself is the route's (`route.rs`),
//! which asks only where the socket is the user's own.
use super::route::{Home, Route};
use serde_json::Value;
use std::time::Duration;

/// Why the control socket gave no answer. On Windows, where it is not
/// asked at all, nothing makes the refusals of a socket file.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Silent {
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

/// Asks the gateway whose home is `home` one question, over `route`.
pub(super) fn ask(
    route: &dyn Route,
    home: &Home,
    line: &Value,
    timeout: Duration,
) -> Result<Value, Silent> {
    let mut request = line.to_string().into_bytes();
    request.push(b'\n');
    let answer = route.exchange(home, &request, timeout)?;
    let line = answer.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
    serde_json::from_slice(line).map_err(|_| Silent::Garbled)
}
