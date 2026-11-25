//! Agent HUD Fork (ticket 78): the inspector's launch steps as the app takes
//! them, in the order `fork::launch` asks for. Nothing here runs unless asked.
use super::{browser, content, fork, hosts, watch, KEEP_DAYS};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread::JoinHandle,
};

type Watch = (mpsc::Sender<()>, JoinHandle<()>);

pub(super) struct Steps<'a> {
    app: &'a tauri::AppHandle,
    data: &'a Path,
    pictures: &'a Path,
    watch: Option<Watch>,
    browser: Option<agent_hud_runtime::local_tools::Server>,
    shortcut: bool,
}

impl<'a> Steps<'a> {
    pub(super) fn new(app: &'a tauri::AppHandle, data: &'a Path, pictures: &'a Path) -> Self {
        Self {
            app,
            data,
            pictures,
            watch: None,
            browser: None,
            shortcut: false,
        }
    }

    /// What launch started: the watch, the relay, and whether the shortcut
    /// is held.
    pub(super) fn started(
        self,
    ) -> (
        Option<Watch>,
        Option<agent_hud_runtime::local_tools::Server>,
        bool,
    ) {
        (self.watch, self.browser, self.shortcut)
    }
}

/// The home folder and this app's executable.
fn home_and_executable() -> Option<(PathBuf, PathBuf)> {
    Some((
        agent_hud_runtime::home_directory().ok()?,
        std::env::current_exe().ok()?,
    ))
}

impl fork::Launch for Steps<'_> {
    fn prune_pictures(&mut self) {
        content::prune(self.pictures, KEEP_DAYS);
    }

    fn write_browser_notes(&mut self) {
        if let Some((home, executable)) = home_and_executable() {
            hosts::install(&home, &executable);
        }
    }

    fn remove_own_browser_notes(&mut self) {
        if let Some((home, executable)) = home_and_executable() {
            fork::remove_own(&home, &executable);
        }
    }

    fn start_watch(&mut self) {
        self.watch = watch::start(self.app.clone());
    }

    fn start_relay(&mut self) {
        self.browser = browser::listen(self.app, self.data);
    }

    fn hold_shortcut(&mut self) {
        self.shortcut = true;
    }
}
