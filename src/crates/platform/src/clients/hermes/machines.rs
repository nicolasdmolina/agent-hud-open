//! Every machine Hermes is followed on: this one, always, and each far
//! machine an entry of the Hermes desktop app's connection list leads
//! to. It starts and stops with client hooks; there is no other switch.
//!
//! - **The list** is looked at on a slow timer and read only when it
//!   changed (`list.rs`). Only a list that parsed changes anything
//!   (`apply.rs`): a file that is missing, unreadable, half written,
//!   broken or of a version that is not understood leaves every machine,
//!   route and card exactly as they were, is said in a note on the whole
//!   list, and is read again at the next look. A list that parsed and
//!   holds no SSH entry does remove them.
//! - **Entries, routes and machines** are kept apart (`book.rs`), and a
//!   route change hands a machine over without ending its cards
//!   (`change.rs`).
//! - **The bindings** remember which machine each entry verified as
//!   (`bindings.rs`). At launch, until a list parses, they show what was
//!   verified before, and nothing is dialled from them.
//! - **The machines list** is told to core after every change, and at
//!   every look, since a link comes and goes without a word (`status.rs`).
use super::{
    route::{local::Local, Route},
    Observer, Pace,
};
use crate::{
    hooks::Wake,
    ssh_link::{Folder, Settings},
};
use agent_hud_agents::hermes::{connections::CONNECTIONS, FAR_ROUTE};
use agent_hud_core::requests::Requests;
use list::{Looked, Watch};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread,
    time::Duration,
};

mod apply;
mod bindings;
mod book;
#[cfg(test)]
mod book_tests;
mod change;
mod keeper;
mod list;
mod status;
pub(in crate::clients::hermes) mod words;

pub(crate) use bindings::Bindings;
pub(crate) use book::{Book, Here, Ways};
#[cfg(test)]
pub(in crate::clients::hermes) use book_tests::Gate;

/// How often the connection list is looked at, and the machines list
/// told again.
const LOOK: Duration = Duration::from_secs(5);
/// The folder that holds every master, in the app's own caches folder:
/// short enough for a socket's path, which the data folder's is not.
const MASTERS: &str = "ssh";

pub struct Machines {
    local: Observer,
    book: Arc<Book>,
    looks: Arc<Looks>,
}

/// Where everything is: the app's own places, or a lab's.
pub(crate) struct Sources {
    /// The route to this machine's Hermes, and which machine that is.
    pub(crate) local: Arc<dyn Route>,
    pub(crate) here: Option<Here>,
    /// The connection list. None where its place is not known.
    pub(crate) list: Option<PathBuf>,
    pub(crate) ways: Result<Ways, &'static str>,
    pub(crate) look: Duration,
    pub(crate) pace: Pace,
}

/// The thread that looks at the list, and what ends it.
struct Looks {
    over: Mutex<bool>,
    ended: Condvar,
    /// Counts the looks, for the tests that wait for one.
    #[cfg(test)]
    looked: std::sync::atomic::AtomicU64,
}

impl Machines {
    /// Follows this machine's Hermes and every far machine the connection
    /// list leads to, logging into `data`.
    pub fn start(data: &Path, requests: Requests, wake: Wake) -> Result<Self, String> {
        let local = Arc::new(Local::new()?);
        let home = crate::os::home_dir()?;
        let environment = std::env::vars().collect();
        let list = CONNECTIONS.on(crate::os::OS).value().and_then(|list| {
            let found = crate::os::resolve(&home, &environment, list);
            found.into_iter().next()
        });
        let ways = match FAR_ROUTE.on(crate::os::OS).value() {
            None => Err("a far Hermes cannot be reached from this system"),
            Some(()) => {
                let caches = crate::os::caches_dir(&home).join(crate::os::DATA_FOLDER);
                let folder = Folder::open(&caches.join(MASTERS));
                let settings = Settings::system();
                folder
                    .map(|folder| Ways { folder, settings })
                    .map_err(|refused| refused.reason())
            }
        };
        let this = local.clone();
        let sources = Sources {
            local,
            here: Some(Arc::new(move || this.key())),
            list,
            ways,
            look: LOOK,
            pace: Pace::default(),
        };
        Self::start_with(sources, data, requests, wake)
    }

    pub(crate) fn start_with(
        sources: Sources,
        data: &Path,
        requests: Requests,
        wake: Wake,
    ) -> Result<Self, String> {
        let (pace, look) = (sources.pace, sources.look);
        let (far, woken) = (requests.clone(), wake.clone());
        let local = Observer::start_on(sources.local, data, requests, wake, pace)?;
        let bindings = Bindings::load(data);
        let book = Book::start(
            (data, pace),
            far,
            woken,
            sources.ways,
            sources.here,
            bindings,
        );
        // What was verified before shows from the start.
        book.tell();
        let looks = Arc::new(Looks {
            over: Mutex::new(false),
            ended: Condvar::new(),
            #[cfg(test)]
            looked: Default::default(),
        });
        let (kept, watched) = (book.clone(), looks.clone());
        let list = sources.list;
        let watching = thread::Builder::new()
            .name("Hermes machines".into())
            .spawn(move || watched.work(&kept, list, look));
        if watching.is_err() {
            book.stop();
            return Err("the Hermes connection list could not be watched".to_owned());
        }
        Ok(Self { local, book, looks })
    }

    /// A Hermes hook reported a turn started, waiting or ended. Hooks run
    /// on this machine: its gateways are polled now.
    pub fn nudge(&self) {
        self.local.nudge();
    }

    #[cfg(test)]
    pub(crate) fn book(&self) -> &Arc<Book> {
        &self.book
    }

    /// How often the list was looked at so far.
    #[cfg(test)]
    pub(crate) fn looks(&self) -> u64 {
        self.looks.looked.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Drop for Machines {
    fn drop(&mut self) {
        *self
            .looks
            .over
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self.looks.ended.notify_all();
        self.book.stop();
    }
}

impl Looks {
    /// Waits `time`, or less once it is over; whether it is.
    fn over(&self, time: Duration) -> bool {
        let over = self.over.lock().unwrap_or_else(PoisonError::into_inner);
        let (over, _) = (self.ended.wait_timeout_while(over, time, |over| !*over))
            .unwrap_or_else(PoisonError::into_inner);
        *over
    }

    fn work(&self, book: &Book, list: Option<PathBuf>, look: Duration) {
        // Before the first route of this launch: every master left in
        // the folder is closed, whichever entry it was started for.
        book.sweep();
        let mut watch = list.map(Watch::new);
        while !self.over(Duration::ZERO) {
            match watch.as_mut().map(Watch::look) {
                Some(Looked::List(list)) => book.apply(list),
                Some(Looked::Same) => book.read(None),
                Some(Looked::Unread(why)) => book.read(Some(why)),
                None => book.read(None),
            }
            #[cfg(test)]
            (self.looked).fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if self.over(look) {
                return;
            }
        }
    }
}
