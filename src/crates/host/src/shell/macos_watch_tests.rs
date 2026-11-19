use super::{Activities, MacOs, Watch, OPTIONS};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObjectProtocol, ProtocolObject},
};
use objc2_foundation::{NSActivityOptions, NSNumber, NSProcessInfo, NSString};
use std::cell::RefCell;

/// Counts what it was asked for, and numbers each activity it hands out.
#[derive(Default)]
struct Counted {
    begun: RefCell<Vec<u32>>,
    ended: RefCell<Vec<u32>>,
}
impl Activities for Counted {
    type Held = u32;
    fn begin(&self) -> u32 {
        let mut begun = self.begun.borrow_mut();
        let number = begun.len() as u32 + 1;
        begun.push(number);
        number
    }
    fn end(&self, held: u32) {
        self.ended.borrow_mut().push(held);
    }
}

#[test]
fn one_activity_is_held_while_the_app_watches() {
    let system = Counted::default();
    let mut watch = Watch(None);
    // Not watching and never was: nothing is asked of the system.
    watch.set(&system, false);
    assert!(system.begun.borrow().is_empty() && system.ended.borrow().is_empty());
    // Every snapshot says it again; the system is asked once.
    for _ in 0..3 {
        watch.set(&system, true);
    }
    assert_eq!(*system.begun.borrow(), [1]);
    assert!(system.ended.borrow().is_empty());
    // Client hooks go off: that same activity is given back, once.
    for _ in 0..3 {
        watch.set(&system, false);
    }
    assert_eq!(*system.ended.borrow(), [1]);
    // And on again: a new one.
    watch.set(&system, true);
    assert_eq!(*system.begun.borrow(), [1, 2]);
    assert_eq!(*system.ended.borrow(), [1]);
}

#[test]
fn the_activity_turns_app_nap_off_and_lets_the_mac_sleep() {
    // Only an activity the user waits on keeps App Nap away: "background"
    // work is what App Nap is for.
    assert!(OPTIONS.contains(NSActivityOptions::UserInitiatedAllowingIdleSystemSleep));
    assert_ne!(
        OPTIONS.0 & !NSActivityOptions::Background.0,
        0,
        "a background activity does not stop App Nap"
    );
    assert!(!OPTIONS.contains(NSActivityOptions::IdleSystemSleepDisabled));
    assert!(!OPTIONS.contains(NSActivityOptions::IdleDisplaySleepDisabled));
    assert!(!OPTIONS.intersects(NSActivityOptions::LatencyCritical));
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// The power assertions every process holds, by pid (a `CFDictionary` of
    /// `CFNumber` to `CFArray`, returned retained).
    fn IOPMCopyAssertionsByProcess(by_process: *mut *mut AnyObject) -> i32;
}

/// How many power assertions (what keeps a Mac or its display awake) this
/// process holds now, as macOS counts them.
fn power_assertions() -> usize {
    let mut by_process: *mut AnyObject = std::ptr::null_mut();
    assert_eq!(unsafe { IOPMCopyAssertionsByProcess(&mut by_process) }, 0);
    let Some(by_process) = (unsafe { Retained::from_raw(by_process) }) else {
        return 0;
    };
    let me = NSNumber::new_i32(std::process::id() as i32);
    let mine: Option<Retained<AnyObject>> = unsafe { msg_send![&*by_process, objectForKey: &*me] };
    mine.map_or(0, |mine| unsafe { msg_send![&*mine, count] })
}

/// The real system, asked for an activity that does keep the Mac awake: the
/// control that shows `power_assertions` sees one.
fn begin_keeping_awake() -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiated,
        &NSString::from_str("Agent HUD's test: an activity that keeps the Mac awake"),
    )
}

#[test]
fn holding_the_real_activity_takes_no_power_assertion() {
    let before = power_assertions();
    let held = MacOs.begin();
    let while_held = power_assertions();
    MacOs.end(held);
    assert_eq!(
        while_held, before,
        "watching for requests must not keep the Mac or its display awake"
    );
    // The same reading does see an activity that keeps the Mac awake. Where
    // the system holds no assertions for anyone (a build machine without a
    // power manager), there is nothing to compare, and the options test
    // above still stands.
    let awake = begin_keeping_awake();
    let with_one = power_assertions();
    unsafe { NSProcessInfo::processInfo().endActivity(&awake) };
    if with_one == before {
        eprintln!("this machine counts no power assertion for an activity that asks for one");
    }
}
