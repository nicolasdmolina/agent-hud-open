//! Start-up cleanup. A crash, or a quit that was cut short, leaves masters
//! running with nobody to close them, and some belong to a machine that is
//! no longer wanted. So the whole folder is gone through, not only the
//! sockets of the machines wanted now.
//!
//! For each socket: its type and owner are read without following a link,
//! its master is asked to exit, the process is confirmed gone, and only
//! then is the socket removed. A master that will not end keeps its
//! socket. Anything that is not this user's socket, record or log is left
//! exactly as it is. A master with no socket yet is found by its record,
//! and one with no record either by the process list.
use super::{
    super::{Settings, Swept},
    args::Ask,
    folder::{entry, user, Folder, LOG, RECORD},
    process,
};
use std::{fs, os::unix::fs::FileTypeExt, os::unix::fs::MetadataExt};

impl Folder {
    /// Run once, before the first link of this launch is opened: it closes
    /// every master in the folder, whoever started it.
    pub fn sweep(&self, settings: &Settings) -> Swept {
        let mut swept = Swept::default();
        let mut names: Vec<String> = fs::read_dir(self.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        for name in &names {
            // Gone since the listing: a master removes its own socket.
            let Ok(node) = fs::symlink_metadata(self.file(name)) else {
                continue;
            };
            let own = node.uid() == user();
            if own && node.file_type().is_socket() {
                match self.close_master(name, settings) {
                    Some(ran) => {
                        swept.closed += usize::from(ran);
                        swept.removed += 1;
                    }
                    None => swept.left += 1,
                }
            } else if !(own && node.is_file() && beside(name).is_some()) {
                swept.left += 1;
            }
        }
        // Records and logs whose socket is gone. A master that had not
        // come up when its owner died has a record and no socket yet.
        for name in names.iter().filter_map(|name| beside(name)) {
            if self.is_socket(name) {
                continue;
            }
            let control = self.control(name);
            if let Some(pid) = self.recorded(name) {
                if process::master_runs(pid, &control) {
                    if !process::end(pid, &control) {
                        swept.left += 1;
                        continue;
                    }
                    swept.closed += 1;
                }
            }
            self.forget(name);
        }
        // Last, by the process list: a master with neither socket nor
        // record, whose owner died in the moment it was started.
        for (pid, name) in process::naming(&self.control_start()) {
            if !entry(&name) {
                continue;
            }
            if process::end(pid, &self.control(&name)) {
                swept.closed += 1;
                self.forget(&name);
            } else {
                swept.left += 1;
            }
        }
        swept
    }

    /// Closes the master of one socket and removes the socket once its
    /// process is confirmed gone. Some(whether a master was running), or
    /// None where one would not end and everything of it was left.
    fn close_master(&self, name: &str, settings: &Settings) -> Option<bool> {
        let control = self.control(name);
        let reported = super::ask(settings, &control, Ask::Check).and_then(|answer| {
            let said = [answer.stdout, answer.stderr].concat();
            reported_pid(&String::from_utf8_lossy(&said))
        });
        if reported.is_some() {
            super::ask(settings, &control, Ask::Exit);
        }
        let mut pids: Vec<u32> = self.recorded(name).into_iter().chain(reported).collect();
        pids.dedup();
        for pid in pids {
            if !process::gone_within(pid, &control, settings.control_limit)
                && !process::end(pid, &control)
            {
                return None;
            }
        }
        self.forget(name);
        Some(reported.is_some())
    }
}

/// The socket name a record or a log belongs to.
fn beside(name: &str) -> Option<&str> {
    name.strip_suffix(RECORD)
        .or_else(|| name.strip_suffix(LOG))
        .filter(|socket| !socket.is_empty())
}

/// The process a master names when asked whether it runs:
/// `Master running (pid=123)`.
fn reported_pid(said: &str) -> Option<u32> {
    let after = said.split("(pid=").nth(1)?;
    let pid: i32 = after.split(')').next()?.trim().parse().ok()?;
    (pid > 1).then_some(pid as u32)
}
