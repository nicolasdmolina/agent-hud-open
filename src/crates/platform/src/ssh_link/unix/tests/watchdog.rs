//! The watchdog's own work (`reaper.rs` says when it runs): which processes
//! are the lab's, ended only while they are still those very processes, and
//! which lab folders are removed. Nothing read from a file or a folder
//! decides any of it (ticket 49, after its second PR verifier).
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// When the kernel says a process began, to the microsecond: with its id,
/// what tells it from a later process given the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Birth(pub(crate) u64, pub(crate) u64);

/// A process now: its id, its parent's, and its birth.
pub(crate) type Process = (u32, u32, Birth);

/// The process `pid` now, as the kernel has it.
#[cfg(target_os = "macos")]
pub(crate) fn process(pid: u32) -> Option<Process> {
    // SAFETY: a zeroed plain struct, filled by the kernel up to its size.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (filled == size).then(|| {
        let birth = Birth(info.pbi_start_tvsec, info.pbi_start_tvusec);
        (pid, info.pbi_ppid, birth)
    })
}

/// The process `pid` now: its parent and its start in clock ticks since
/// boot, from `/proc`.
#[cfg(not(target_os = "macos"))]
pub(crate) fn process(pid: u32) -> Option<Process> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let parent = fields.get(1)?.parse().ok()?;
    let start = fields.get(19)?.parse().ok()?;
    Some((pid, parent, Birth(start, 0)))
}

/// Every process now that the kernel tells this user of.
#[cfg(target_os = "macos")]
pub(crate) fn processes() -> Vec<Process> {
    let mut pids = vec![0 as libc::pid_t; 8192];
    loop {
        let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: the buffer is `bytes` long.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if count < 0 {
            return Vec::new();
        }
        if (count as usize) < pids.len() {
            pids.truncate(count as usize);
            break;
        }
        pids.resize(pids.len() * 2, 0);
    }
    pids.into_iter()
        .filter(|pid| *pid > 0)
        .filter_map(|pid| process(pid as u32))
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn processes() -> Vec<Process> {
    let Ok(listed) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    listed
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .filter_map(process)
        .collect()
}

/// The lab's processes, each by its id and birth.
#[derive(Default)]
pub(crate) struct Registry {
    known: HashMap<u32, Birth>,
}

impl Registry {
    /// Notes `pid` of `birth`. The system's first process (id 1) and none
    /// below it is ever noted: all the others descend from it.
    pub(crate) fn add(&mut self, pid: u32, birth: Birth) {
        if pid > 1 {
            self.known.insert(pid, birth);
        }
    }

    /// Whether `pid` is a process of the registry: the one it noted, not a
    /// later one given its id.
    pub(crate) fn holds(&self, (pid, _, birth): &Process) -> bool {
        self.known.get(pid) == Some(birth)
    }

    /// Adds every process below one of the registry's, but `except`: born
    /// to a process that is the registry's own, and not before it. The
    /// parent is looked at again, as the kernel has it now: a listing is
    /// taken one process at a time, so its line for the parent can be older
    /// than its line for the child, from before the parent's id went to
    /// another process (the third PR verifier's mixed snapshot).
    pub(crate) fn grow(&mut self, now: &[Process], except: u32) {
        loop {
            let mut added = false;
            for (pid, parent, birth) in now {
                if *pid <= 1 || *pid == except || self.known.get(pid) == Some(birth) {
                    continue;
                }
                let Some(noted) = self.known.get(parent).copied() else {
                    continue;
                };
                if *birth < noted || process(*parent).map(|now| now.2) != Some(noted) {
                    continue;
                }
                self.known.insert(*pid, *birth);
                added = true;
            }
            if !added {
                return;
            }
        }
    }

    /// Stops and kills every process of the registry, and every one below,
    /// but `except` and another lab program's watchdog (`spared`), until
    /// none is left. Each signal goes only to a process whose birth, looked
    /// at just before, is the one noted.
    pub(crate) fn reap(&mut self, except: u32) {
        for _ in 0..50 {
            let now = processes();
            self.grow(&now, except);
            let live: Vec<(u32, Birth)> = now
                .iter()
                .filter(|process| process.0 != except && self.holds(process))
                .filter(|process| !spared(process.0))
                .map(|(pid, _, birth)| (*pid, *birth))
                .collect();
            if live.is_empty() {
                return;
            }
            for signal in [libc::SIGSTOP, libc::SIGKILL] {
                for (pid, birth) in &live {
                    if process(*pid).map(|now| now.2) == Some(*birth) {
                        // SAFETY: a signal to a process of the lab's own,
                        // checked by its birth just now.
                        unsafe { libc::kill(*pid as libc::pid_t, signal) };
                    }
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The identity of what is at `path` itself (a link is not followed).
pub(crate) fn identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

/// A lab folder the watchdog holds open, with its identity.
pub(crate) struct Held {
    path: PathBuf,
    identity: (u64, u64),
    folder: OwnedFd,
}

impl Held {
    /// The folder at `path`, held where it is a folder (not a link) of the
    /// identity the lab program gave.
    pub(crate) fn open(path: &Path, identity: (u64, u64)) -> Option<Self> {
        let name = CString::new(path.as_os_str().as_bytes()).ok()?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: a plain open of a path; the descriptor is owned below.
        let raw = unsafe { libc::open(name.as_ptr(), flags) };
        if raw < 0 {
            return None;
        }
        // SAFETY: `raw` was opened just now and is owned only here.
        let folder = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: a zeroed plain struct for `fstat` to fill.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(folder.as_raw_fd(), &mut stat) } != 0 {
            return None;
        }
        let held = (stat.st_dev as u64, stat.st_ino);
        (held == identity).then(|| Self {
            path: path.to_owned(),
            identity,
            folder,
        })
    }

    /// Empties the folder through its own descriptor, never following a
    /// link, then removes its path where that path is still this folder.
    pub(crate) fn remove(&self) {
        clear(self.folder.as_raw_fd());
        if identity(&self.path) == Some(self.identity) {
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

/// Removes everything in the folder open as `folder`, folders within it
/// through their own descriptors, and links as links.
fn clear(folder: RawFd) {
    let mut names = Vec::new();
    // SAFETY: a duplicate for `fdopendir` to own; it is closed with the
    // stream, and the entries are copied out before that.
    unsafe {
        let copy = libc::dup(folder);
        if copy < 0 {
            return;
        }
        let stream = libc::fdopendir(copy);
        if stream.is_null() {
            libc::close(copy);
            return;
        }
        loop {
            let entry = libc::readdir(stream);
            if entry.is_null() {
                break;
            }
            let name = CStr::from_ptr((*entry).d_name.as_ptr()).to_owned();
            if name.as_bytes() != b"." && name.as_bytes() != b".." {
                names.push(name);
            }
        }
        libc::closedir(stream);
    }
    for name in names {
        // SAFETY: calls relative to the open folder, never following a link.
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            if libc::fstatat(folder, name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) != 0 {
                continue;
            }
            if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
                let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                let inner = libc::openat(folder, name.as_ptr(), flags);
                if inner >= 0 {
                    clear(inner);
                    libc::close(inner);
                }
                libc::unlinkat(folder, name.as_ptr(), libc::AT_REMOVEDIR);
            } else {
                libc::unlinkat(folder, name.as_ptr(), 0);
            }
        }
    }
}

/// Whether `pid` is another watchdog: this same test program, leading a
/// process group of its own, which only a watchdog does (`reaper.rs`). A
/// lab program started below a lab program has its own, which ends its
/// lab when its program goes: this one leaves it to that (the third PR
/// verifier's nested watchdogs).
#[cfg(target_os = "macos")]
fn spared(pid: u32) -> bool {
    let path = |pid: u32| {
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is as long as it says.
        let length = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        (length > 0).then(|| buffer[..length as usize].to_vec())
    };
    // SAFETY: a zeroed plain struct, filled by the kernel up to its size.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    filled == size
        && info.pbi_pgid == pid
        && path(pid).is_some()
        && path(pid) == path(std::process::id())
}

#[cfg(not(target_os = "macos"))]
fn spared(pid: u32) -> bool {
    let group = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(2)?
                .parse::<u32>()
                .ok()
        });
    let path = |of: &str| std::fs::read_link(format!("/proc/{of}/exe")).ok();
    group == Some(pid) && path(&pid.to_string()).is_some() && path(&pid.to_string()) == path("self")
}

/// The owner, from an `owner PID SECONDS MICROS` line: the lab program as
/// it found itself before it started this watchdog, where it is still that
/// process. Never the system's first process.
pub(crate) fn owner(line: &str) -> Option<(u32, Birth)> {
    let mut parts = line.strip_prefix("owner ")?.split(' ');
    let pid: u32 = parts.next()?.parse().ok()?;
    let birth = Birth(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    (pid > 1 && process(pid)?.2 == birth).then_some((pid, birth))
}

/// The watchdog. Its owner comes first on `input`, the lab program's
/// private pipe, as the program found itself before it started this
/// watchdog (never this process's parent, which can already be the system
/// by now); `ready` goes back on `output`. Then come the processes the lab
/// program starts (`pid PID SECONDS MICROS`), the lab folders it asks for
/// (`make BASE`, made here and answered `made PATH` or `refused`) and those
/// a test made itself (`dir DEV INO PATH`, answered `held` or `refused`). Every ~200 ms it adds the
/// processes below the registry's, until the pipe closes or the owner is
/// gone; then it ends the lab's processes and removes its folders, all
/// that was sent, also where the owner was gone before it began.
pub(crate) fn run(input: impl Read + Send + 'static, mut output: impl Write) -> ! {
    let me = std::process::id();
    let mut lines = BufReader::new(input).lines();
    let owner = lines
        .next()
        .and_then(Result::ok)
        .and_then(|line| owner(&line));
    let mut registry = Registry::default();
    if let Some((pid, birth)) = owner {
        registry.add(pid, birth);
    }
    let _ = writeln!(output, "ready");
    let _ = output.flush();
    let (told, heard) = mpsc::channel();
    thread::spawn(move || {
        for line in lines {
            let Ok(line) = line else { break };
            if told.send(Some(line)).is_err() {
                return;
            }
        }
        let _ = told.send(None);
    });
    let mut held = Vec::new();
    let mut take = |line: String, registry: &mut Registry, output: &mut dyn Write| {
        if let Some((pid, birth)) = seed(&line) {
            registry.add(pid, birth);
        } else if line.starts_with("dir ") {
            let folder = folder(&line);
            let said = if folder.is_some() { "held" } else { "refused" };
            held.extend(folder);
            let _ = writeln!(output, "{said}");
            let _ = output.flush();
        } else if let Some(base) = line.strip_prefix("make ") {
            let folder = make(Path::new(base));
            let said = folder.as_ref().map_or_else(
                || "refused".to_owned(),
                |folder| format!("made {}", folder.path.display()),
            );
            held.extend(folder);
            let _ = writeln!(output, "{said}");
            let _ = output.flush();
        }
    };
    let mut ended = false;
    let mut gone_since: Option<Instant> = None;
    loop {
        let until = Instant::now() + Duration::from_millis(200);
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match heard.recv_timeout(left) {
                Ok(Some(line)) => take(line, &mut registry, &mut output),
                Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    ended = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
            }
        }
        if ended {
            break;
        }
        let now = processes();
        registry.grow(&now, me);
        let here = owner.is_some_and(|(pid, _)| {
            now.iter()
                .any(|process| process.0 == pid && registry.holds(process))
        });
        // An owner that is gone has said all it will: what it sent is read
        // to the end of the pipe, or for two seconds at most.
        if !here && gone_since.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(2) {
            break;
        }
    }
    registry.reap(me);
    for folder in &held {
        folder.remove();
    }
    std::process::exit(0)
}

/// A `pid PID SECONDS MICROS` line: a process the lab program started,
/// where it is still that process.
fn seed(line: &str) -> Option<(u32, Birth)> {
    let mut parts = line.strip_prefix("pid ")?.split(' ');
    let pid = parts.next()?.parse().ok()?;
    let birth = Birth(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    (process(pid)?.2 == birth).then_some((pid, birth))
}

/// A new lab folder in `base`, made here and held before anyone is told
/// its name (`make BASE`). One that cannot be held is removed again where
/// it is still the empty folder made.
pub(crate) fn make(base: &Path) -> Option<Held> {
    let template = base.join(format!("{}XXXXXX", super::reaper::PREFIX));
    let mut name = CString::new(template.as_os_str().as_bytes())
        .ok()?
        .into_bytes_with_nul();
    // SAFETY: a writable template ending in its nul, filled in place.
    if unsafe { libc::mkdtemp(name.as_mut_ptr().cast()) }.is_null() {
        return None;
    }
    name.pop();
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&name));
    let held = identity(&path).and_then(|made| Held::open(&path, made));
    if held.is_none() {
        let _ = std::fs::remove_dir(&path);
    }
    held
}

/// A `dir DEV INO PATH` line, held.
fn folder(line: &str) -> Option<Held> {
    let rest = line.strip_prefix("dir ")?;
    let mut parts = rest.splitn(3, ' ');
    let dev = parts.next()?.parse().ok()?;
    let ino = parts.next()?.parse().ok()?;
    Held::open(Path::new(parts.next()?), (dev, ino))
}
