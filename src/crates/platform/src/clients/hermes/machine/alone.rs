//! One far machine given directly, with no connection list: the route to
//! one address, and the machine it verifies as. It is the book with one
//! entry (`machines/book.rs`), so what runs here is what runs for an
//! entry of the list.
use super::{FarEntry, Inner, Standing};
use crate::{
    clients::hermes::{
        machines::{Bindings, Book, Ways},
        Pace,
    },
    hooks::Wake,
};
use agent_hud_core::requests::Requests;
use std::{path::Path, sync::Arc};

pub struct FarObserver {
    book: Arc<Book>,
    inner: Arc<Inner>,
}

impl FarObserver {
    /// Follows the gateways of the far machine at `entry`, logging into
    /// `data`. The link is opened in the background: this returns at once.
    pub fn start(
        entry: FarEntry,
        data: &Path,
        requests: Requests,
        wake: Wake,
    ) -> Result<Self, String> {
        Self::start_at(entry, data, requests, wake, Pace::default())
    }

    pub(crate) fn start_at(
        entry: FarEntry,
        data: &Path,
        requests: Requests,
        wake: Wake,
        pace: Pace,
    ) -> Result<Self, String> {
        let FarEntry {
            target,
            label,
            folder,
            settings,
        } = entry;
        let ways = Ok(Ways { folder, settings });
        // Nobody reads the mirrors of a machine followed this way.
        let untold: crate::hooks::Changed = Arc::new(|_| {});
        let book = Book::start(
            (data, pace),
            requests,
            (wake, untold),
            ways,
            None,
            Bindings::default(),
        );
        match book.given(&label, target) {
            Ok(inner) => Ok(Self { book, inner }),
            Err(why) => {
                book.stop();
                Err(why)
            }
        }
    }

    /// How the machine stands now.
    pub fn standing(&self) -> Standing {
        self.inner.standing()
    }

    /// The gateways found without the live connection, each by its
    /// profile with Hermes's reason. None of them is followed.
    pub fn not_followed(&self) -> Vec<(String, &'static str)> {
        self.inner.not_followed()
    }

    #[cfg(test)]
    pub(in crate::clients::hermes) fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }
}

impl Drop for FarObserver {
    fn drop(&mut self) {
        self.book.stop();
    }
}
