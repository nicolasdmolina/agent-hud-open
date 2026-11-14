//! Asking who is there: the home a follower asks, the identify it sends,
//! and what the asking tells the follower of its own gateway. It is gone
//! where nobody listens at its control socket, or where every home of
//! the machine was read and its home is not among them. It is another
//! gateway's where another instance answers (`link.rs`, `settle.rs`).
use super::{
    control::{self, Silent},
    route::Home,
    Shared,
};
use agent_hud_agents::hermes::gateway::{identify, identify_line, Identified};
use serde_json::Value;
use std::sync::{atomic::Ordering, PoisonError};

/// Where the home a follower asks is, by one reading of the machine's
/// homes.
enum Listed {
    Home(Home),
    /// Every home there is was read, and it is not among them.
    Gone,
    /// It is not among the homes that could be read, and not all could:
    /// that says nothing of it.
    Untold,
}

/// The home whose gateway is followed, as it is now: the one this
/// follower asks, among the homes the route tells of. An error where the
/// homes cannot be told at all.
fn listed(shared: &Shared) -> Result<Listed, Silent> {
    let (told, asked) = (shared.route.inventory()?, shared.asked());
    let mut homes = told.homes.into_iter();
    Ok(match homes.find(|home| *home.profile() == asked) {
        Some(home) => Listed::Home(home),
        None if told.whole => Listed::Gone,
        None => Listed::Untold,
    })
}

/// The home whose gateway is followed.
pub(super) fn home(shared: &Shared) -> Result<Home, Silent> {
    match listed(shared)? {
        Listed::Home(home) => Ok(home),
        Listed::Gone => Err(Silent::Absent),
        Listed::Untold => Err(Silent::Unclear),
    }
}

/// The answer this follower was made from, until its first identify
/// takes it.
pub(super) fn born(shared: &Shared) -> std::sync::MutexGuard<'_, Option<Value>> {
    shared.born.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asks the control socket of that home who is there, once more where it
/// only was slow: identify can stall on a busy gateway and answer at once
/// the next time. The answer comes with the home that was asked. A
/// follower made from an answer takes that one first.
///
/// This is where a follower learns that its gateway is gone: its socket
/// is lost, or it would not be asking, and either nobody listens at the
/// control socket of its home, or every home of the machine was read and
/// its home is not among them. A machine whose homes cannot be told, or
/// not all of them (a `profiles` folder that cannot be read), and a
/// control socket that is there and does not answer, or cannot be asked,
/// say no such thing. The mark changes only with what the asking
/// found, and never in between.
pub(super) fn asked(shared: &Shared) -> Result<(Value, Home), Silent> {
    let home = match listed(shared)? {
        Listed::Home(home) => home,
        Listed::Gone => {
            shared.gone.store(true, Ordering::Release);
            return Err(Silent::Absent);
        }
        Listed::Untold => return Err(Silent::Unclear),
    };
    if let Some(answer) = born(shared).take() {
        return Ok((answer, home));
    }
    let line = identify_line();
    let ask = || control::ask(&*shared.route, &home, &line, shared.pace.answer);
    let answer = match ask() {
        // Asked once more where it only was slow, by a follower that
        // still follows: one that ended meanwhile asks nothing more.
        Err(Silent::TimedOut) if shared.active() => ask(),
        answer => answer,
    };
    let nobody = matches!(answer, Err(Silent::Absent));
    let was = shared.gone.swap(nobody, Ordering::AcqRel);
    if was && !nobody {
        // Back from gone: it claims its profiles again, and polls again
        // once a look that begins from now on has settled what that
        // means for the cards it may open.
        shared.awaits.store(shared.look.next(), Ordering::Release);
        shared.look.want();
    }
    Ok((answer?, home))
}

/// Whether `answer` is another instance than the one this follower
/// follows. It is kept, and a look at the whole machine is asked for.
pub(super) fn elsewhere(shared: &Shared, answer: &Value, home: &str) -> bool {
    let Identified::Ready(now) = identify(answer, home) else {
        return false;
    };
    let follows = |bound: &str| bound != now.id;
    if !shared.model().gateway.instance().is_some_and(follows) {
        return false;
    }
    *shared.found.lock().unwrap_or_else(PoisonError::into_inner) = Some(now.id);
    shared.look.want();
    true
}
