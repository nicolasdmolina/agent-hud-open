//! A dropped lock is free at once, also while a process this one started
//! still holds a copy of the lock's file.
use super::*;
use std::process::{Command, Stdio};

#[test]
fn a_dropped_lock_is_free_while_a_started_process_holds_its_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data/instance.lock");
    let Claim::Acquired(first) = claim(&path, None) else {
        panic!("the first claim takes the lock");
    };
    // A process being started holds a copy of every open file until it runs
    // its program. This one keeps its copy for as long as it runs.
    let copy = first.file.try_clone().unwrap();
    let mut child = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::from(copy))
        .spawn()
        .unwrap();
    drop(first);
    let next = claim(&path, None);
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        matches!(next, Claim::Acquired(_)),
        "the lock is free once its holder lets go"
    );
}
