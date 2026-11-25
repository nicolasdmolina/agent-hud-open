//! The asks a session's channel says wait in it, the ones it shows no card
//! for included (`Requests::native_wait_asks`, Agent HUD Fork, ticket 74).
//! Each ask is a request waiting for the person, and it sounds once, under
//! a name of its own, unless its card sounds for it or it is approved. Two
//! requests that wait one after the other in one session, with nothing
//! between that says the session is at work, are two asks, and each
//! sounds whichever of them shows a card. A request the channel asks anew
//! (it was answered, lost from sight and is seen waiting again) comes under
//! another ask, so it sounds again; one only withdrawn and restored is the
//! same ask, and does not.
use super::{Tracker, Wait};

/// One ask at this look: the request's id, the ask's own name from its
/// channel, and the name it is decided under.
#[derive(Clone)]
pub(super) struct Ask {
    pub request: String,
    pub ask: String,
    pub name: String,
}

impl Tracker {
    /// The name each ask in the session goes by at this look. An ask the
    /// look `before` held whose request waits in no ask now is over, so
    /// the same ask waiting again later goes by the name after it and
    /// sounds again, as a wait that names no request does (`bare.rs`). A
    /// request asked anew is not over: its name stays as it was, and so
    /// does the name its wait has where its channel names it. Nor is an
    /// ask whose name is that of a request its channel still `named` for
    /// the wait: that wait goes on, and so does its name.
    pub(super) fn ask_names(
        &mut self,
        before: Option<&Wait>,
        asks: &[(String, String)],
        named: &[String],
        session: &str,
        now: i64,
    ) -> Vec<Ask> {
        let base = |ask: &String| format!("needs-you:{session}:{ask}");
        for old in before.into_iter().flat_map(|wait| &wait.asks) {
            let gone = !asks.iter().any(|(request, _)| *request == old.request);
            if gone && !named.contains(&old.ask) {
                self.spent(before, &base(&old.ask));
                self.end(Some(base(&old.ask)), now);
            }
        }
        let named = asks.iter().map(|(request, ask)| Ask {
            request: request.clone(),
            ask: ask.clone(),
            name: self.named(&base(ask), now),
        });
        named.collect()
    }

    /// The name of the first ask that is new at this look and sounds:
    /// every ask but those `quiet` says its card or its approval stands
    /// for. Each one sounding is decided now, sounded or not.
    pub(super) fn sound_asks(
        &mut self,
        asks: &[Ask],
        quiet: impl Fn(&Ask) -> bool,
        now: i64,
    ) -> Option<String> {
        let mut told = None;
        for ask in asks.iter().filter(|ask| !quiet(ask)) {
            if self.first(&ask.name, now) {
                told.get_or_insert(ask.name.clone());
            }
        }
        told
    }
}
