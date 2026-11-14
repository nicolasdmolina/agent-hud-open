//! A transcript of every command a link ran, for a test that reads a
//! real far machine and has to say exactly what it did there. Off unless
//! a test turns it on: then each command is kept with how it ended, how
//! long it took and how much it printed, never what it printed.
use crate::ssh_link::{Failure, Ran};
use std::{
    sync::{Mutex, PoisonError},
    time::Duration,
};

static KEPT: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Keeps a transcript from now on.
pub(crate) fn keep() {
    *KEPT.lock().unwrap_or_else(PoisonError::into_inner) = Some(Vec::new());
}

/// Every command so far, in order.
pub(crate) fn kept() -> Vec<String> {
    let kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    kept.clone().unwrap_or_default()
}

/// One command, as it went to the far shell, and what came of it.
pub(crate) fn ran(line: &str, came: &Result<Ran, Failure>, took: Duration) {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(kept) = kept.as_mut() else {
        return;
    };
    // The program a command reads is not on its line, and a line is
    // short by the link's own rule; a long one is cut all the same.
    let line: String = line.chars().take(600).collect();
    let ended = match came {
        Ok(ran) => format!("{:?} stdout_bytes={}", ran.end, ran.stdout.len()),
        Err(failure) => format!("not run: {failure}"),
    };
    let number = kept.len() + 1;
    let ms = took.as_millis();
    kept.push(format!("{number}. {line}\n   -> {ended} ms={ms}"));
}
