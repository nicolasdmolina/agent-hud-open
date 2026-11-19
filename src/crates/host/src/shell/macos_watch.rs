//! macOS: while the app watches for agents' requests it holds one activity
//! with the system. Without one, App Nap takes an app none of whose windows
//! can be seen (a locked screen is enough) down to priority 4 about half a
//! minute later, every thread of it. A Mac busy with builds and agents then
//! starves the followers' polls, the hook inbox and the sounds for tens of
//! seconds, which is exactly when nobody is looking at the HUD. With the
//! activity held macOS still lowers such an app, but only to priority 20,
//! where the same load was measured to leave polls and cards within a second
//! (FORK.md, "App Nap, and the priority the app watches at").
use objc2::{rc::Retained, runtime::NSObjectProtocol, runtime::ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
use std::sync::{Mutex, PoisonError};

/// Work the user waits on, so App Nap leaves the app alone. The Mac and its
/// display still go to sleep when idle: this holds no power assertion.
/// Adding `LatencyCritical` would keep the app at its normal priority, 46,
/// by also asking for exact timers; the measurements did not need it.
const OPTIONS: NSActivityOptions = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep;
/// What Activity Monitor and the system's logs say the activity is for.
const REASON: &str = "Watching for agents' requests";

/// Who holds activities for the app: macOS, or a test.
trait Activities {
    type Held;
    fn begin(&self) -> Self::Held;
    fn end(&self, held: Self::Held);
}

/// The one activity, held exactly while the app watches.
struct Watch<H>(Option<H>);
impl<H> Watch<H> {
    fn set<A: Activities<Held = H>>(&mut self, system: &A, on: bool) {
        match (on, self.0.take()) {
            (true, None) => self.0 = Some(system.begin()),
            (true, held) => self.0 = held,
            (false, Some(held)) => system.end(held),
            (false, None) => {}
        }
    }
}

struct MacOs;
struct Held(Retained<ProtocolObject<dyn NSObjectProtocol>>);
// The token is only kept and handed back to `endActivity`, and
// `NSProcessInfo` may be called from any thread.
unsafe impl Send for Held {}
impl Activities for MacOs {
    type Held = Held;
    fn begin(&self) -> Held {
        let reason = NSString::from_str(REASON);
        Held(NSProcessInfo::processInfo().beginActivityWithOptions_reason(OPTIONS, &reason))
    }
    fn end(&self, held: Held) {
        unsafe { NSProcessInfo::processInfo().endActivity(&held.0) };
    }
}

static WATCH: Mutex<Watch<Held>> = Mutex::new(Watch(None));

/// Holds the activity while `on`, and gives it back when not. Telling it the
/// same thing again changes nothing.
pub(super) fn set(on: bool) {
    WATCH
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .set(&MacOs, on);
}

#[cfg(test)]
#[path = "macos_watch_tests.rs"]
mod tests;
