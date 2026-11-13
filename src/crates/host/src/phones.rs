//! The host's own sharing with paired phones: quota readings and token usage
//! over the LAN link, for a run with no product that shares instead.
use crate::{backend::Backend, extension::ProductExtension};
use agent_hud_share::PhoneShare;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, Weak},
};

/// Agent HUD Fork (ticket 08, FORK.md): phone sharing never starts, so no
/// LAN port opens; the LAN and share code stay as upstream has them.
const SHARES_WITH_PHONES: bool = false;

/// `directory` is None for the demo, which shares nothing.
pub(crate) fn start(
    directory: Option<PathBuf>,
    extension: Option<&dyn ProductExtension>,
    backend: Weak<Mutex<Backend>>,
) -> Option<Arc<PhoneShare>> {
    if !SHARES_WITH_PHONES {
        return None;
    }
    let directory = directory?;
    if extension.is_some_and(|product| product.shares_with_phones()) {
        return None;
    }
    let query = Arc::new(move |method: &str, params| {
        let backend = backend.upgrade().ok_or("Application is stopping")?;
        let mut backend = backend.lock().unwrap_or_else(PoisonError::into_inner);
        backend.runtime.local_query(method, params)
    });
    match PhoneShare::start(directory, query) {
        Ok(share) => Some(Arc::new(share)),
        Err(error) => {
            eprintln!("Sharing with phones could not start: {error}");
            None
        }
    }
}
