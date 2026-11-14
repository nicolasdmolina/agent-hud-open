//! The SSH link on Windows. Not implemented: Windows' OpenSSH has no
//! control sockets, so one owned master cannot carry commands and forwards.
//! An adapter here would hold one `ssh` process for each forward and each
//! command, under a job object. Until then every entry point says so, and
//! no folder is made and no process is started.
use super::{
    Closed, Failure, FarCommand, Forward, Lost, Ran, Refused, Settings, Swept, Target, Watch,
};
use std::{net::SocketAddr, path::Path};

pub const PROGRAM: &str = "ssh.exe";
const NOT_IMPLEMENTED: Failure = Failure::Refused(Refused::NotImplemented);

#[derive(Clone, Debug)]
pub struct Folder(());

impl Folder {
    pub fn open(_path: &Path) -> Result<Self, Refused> {
        Err(Refused::NotImplemented)
    }
    pub fn path(&self) -> &Path {
        Path::new("")
    }
    pub fn sweep(&self, _settings: &Settings) -> Swept {
        Swept::default()
    }
}

pub struct Link(());

impl Link {
    pub fn open(_folder: &Folder, _target: &Target, _settings: &Settings) -> Result<Self, Failure> {
        Err(NOT_IMPLEMENTED)
    }
    pub fn run_watched(&self, _command: &FarCommand, _watch: Watch<'_>) -> Result<Ran, Failure> {
        Err(NOT_IMPLEMENTED)
    }
    pub fn run(&self, _command: &FarCommand) -> Result<Ran, Failure> {
        Err(NOT_IMPLEMENTED)
    }
    pub fn forward(&self, _far: SocketAddr) -> Result<Forward, Failure> {
        Err(NOT_IMPLEMENTED)
    }
    pub fn cancel(&self, _forward: Forward) -> Result<(), Failure> {
        Err(NOT_IMPLEMENTED)
    }
    pub fn alive(&self) -> Result<(), Lost> {
        Err(Lost::Ended)
    }
    pub fn master_pid(&self) -> Option<u32> {
        None
    }
    pub fn close(&self) -> Closed {
        Closed {
            exit: None,
            cancelled: 0,
            gone: true,
        }
    }
}
