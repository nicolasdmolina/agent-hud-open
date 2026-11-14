//! The one private folder that holds every master's control socket. Beside
//! each socket `<name>` are `<name>.pid`, the record naming the master's
//! process, and `<name>.log`, what the master wrote.
//!
//! The folder is this user's own, closed to everyone else, and never a
//! link. Its path is short enough for a socket. Every removal names one
//! entry of the folder and removes that entry itself, so nothing outside
//! the folder is ever unlinked and a link in it is never followed.
use super::super::{Refused, LONGEST_SOCKET_PATH};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// The length of a socket's name in the folder.
const NAME: usize = 8;
pub(super) const RECORD: &str = ".pid";
pub(super) const LOG: &str = ".log";

#[derive(Clone, Debug)]
pub struct Folder {
    path: PathBuf,
}

impl Folder {
    /// The folder at `path`, made where absent. Refused: a path too long
    /// for a socket in it, a link, another user's folder, and one that is
    /// open to anyone else. Nothing is repaired: a folder someone opened up
    /// is not trusted again by closing it.
    pub fn open(path: &Path) -> Result<Self, Refused> {
        let plain = |text: &&str| {
            !text
                .chars()
                .any(|c| c.is_control() || matches!(c, '"' | '\\' | '$'))
        };
        let text = path
            .to_str()
            .filter(|_| path.is_absolute())
            .filter(plain)
            .ok_or(Refused::FolderPath)?;
        if text.len() + 1 + NAME > LONGEST_SOCKET_PATH {
            return Err(Refused::PathTooLong);
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| Refused::Unavailable)?;
        let node = fs::symlink_metadata(path).map_err(|_| Refused::Unavailable)?;
        if !node.is_dir() {
            return Err(Refused::FolderKind);
        }
        if node.uid() != user() {
            return Err(Refused::FolderOwner);
        }
        if node.mode() & 0o777 != 0o700 {
            return Err(Refused::FolderMode);
        }
        Ok(Self {
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One entry of the folder. `name` is a single path component.
    pub(super) fn file(&self, name: &str) -> PathBuf {
        debug_assert!(entry(name));
        self.path.join(name)
    }

    /// The socket path as `ssh` reads it in `ControlPath=`: in quotes, as
    /// the folder may hold a space, with `%` doubled so no token is read.
    pub(super) fn control(&self, name: &str) -> String {
        debug_assert!(entry(name));
        format!("{}{}\"", self.control_start(), name.replace('%', "%%"))
    }

    /// What every control path in this folder starts with.
    pub(super) fn control_start(&self) -> String {
        format!("\"{}/", self.path.display()).replace('%', "%%")
    }

    /// A name no entry of the folder has.
    pub(super) fn fresh(&self) -> Result<String, Refused> {
        for _ in 0..16 {
            let name = uuid::Uuid::new_v4().simple().to_string()[..NAME].to_owned();
            if [
                name.clone(),
                format!("{name}{RECORD}"),
                format!("{name}{LOG}"),
            ]
            .iter()
            .all(|entry| fs::symlink_metadata(self.file(entry)).is_err())
            {
                return Ok(name);
            }
        }
        Err(Refused::Unavailable)
    }

    /// The file the master of `name` writes to. It is opened to append,
    /// so the file can be emptied under a master that still writes.
    pub(super) fn log(&self, name: &str) -> io::Result<fs::File> {
        crate::os::private_append(&self.file(&format!("{name}{LOG}")))
    }

    /// Empties the log of `name` once it is long. A master that stays up
    /// for weeks adds a line for every connection a forward could not
    /// make, and only its last words are ever read.
    pub(super) fn trim(&self, name: &str) {
        const LONG: u64 = 256 * 1024;
        let log = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.file(&format!("{name}{LOG}")));
        if let Ok(log) = log {
            let long = log
                .metadata()
                .is_ok_and(|node| node.is_file() && node.uid() == user() && node.len() > LONG);
            if long {
                let _ = log.set_len(0);
            }
        }
    }

    /// The end of what the master of `name` wrote.
    pub(super) fn said(&self, name: &str) -> String {
        const MOST: u64 = 8 * 1024;
        let Ok(mut log) = self.own_file(&format!("{name}{LOG}")) else {
            return String::new();
        };
        let size = log.metadata().map(|node| node.len()).unwrap_or(0);
        let mut text = Vec::new();
        let _ = io::Seek::seek(&mut log, io::SeekFrom::Start(size.saturating_sub(MOST)));
        let _ = log.take(MOST).read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    }

    /// Writes the record naming the master's process.
    pub(super) fn record(&self, name: &str, pid: u32) -> io::Result<()> {
        let mut record = crate::os::private_file(&self.file(&format!("{name}{RECORD}")), true)?;
        writeln!(record, "{pid}")
    }

    /// The process the record of `name` names.
    pub(super) fn recorded(&self, name: &str) -> Option<u32> {
        let mut text = String::new();
        self.own_file(&format!("{name}{RECORD}"))
            .ok()?
            .take(32)
            .read_to_string(&mut text)
            .ok()?;
        let pid: i32 = text.trim().parse().ok()?;
        (pid > 1).then_some(pid as u32)
    }

    /// Whether `name` is a socket of this user's. A link is not followed.
    pub(super) fn is_socket(&self, name: &str) -> bool {
        fs::symlink_metadata(self.file(name))
            .is_ok_and(|node| node.file_type().is_socket() && node.uid() == user())
    }

    /// Removes the socket, the record and the log of `name`. The caller
    /// has confirmed the master gone.
    pub(super) fn forget(&self, name: &str) {
        if self.is_socket(name) {
            self.unlink(name);
        }
        for beside in [format!("{name}{RECORD}"), format!("{name}{LOG}")] {
            let plain = fs::symlink_metadata(self.file(&beside))
                .is_ok_and(|node| node.is_file() && node.uid() == user());
            if plain {
                self.unlink(&beside);
            }
        }
    }

    /// Removes one entry of the folder: the entry itself, never what a
    /// link leads to, and never a path with a folder in it.
    fn unlink(&self, name: &str) {
        if entry(name) {
            let _ = fs::remove_file(self.path.join(name));
        }
    }

    /// Opens a plain file of this user's in the folder, following no link.
    fn own_file(&self, name: &str) -> io::Result<fs::File> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.file(name))?;
        let node = file.metadata()?;
        if !node.is_file() || node.uid() != user() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(file)
    }
}

/// Whether `name` is one entry's name and not a path.
pub(super) fn entry(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/')
}

pub(super) fn user() -> u32 {
    // SAFETY: reads this process's own user id.
    unsafe { libc::geteuid() }
}
