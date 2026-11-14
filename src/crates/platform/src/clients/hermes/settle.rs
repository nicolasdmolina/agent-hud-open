//! What a look at the machine changes (`discovery.rs`). Only what the
//! look verified counts, so a look that reached no home, or only some,
//! leaves every follower and every card it did not hear of as they were.
//!
//! A follower ends for one of these, and for nothing else:
//!
//! - Its home answers as another instance, which the follower found for
//!   itself, and its own instance was heard nowhere: the gateway
//!   restarted. Its cards close as a restart, and the gateway that
//!   answers now gets a follower of its own, its anchor picked again.
//! - Every profile it held is another instance's now.
//! - Its id is the name of a profile it serves no more, and the gateway
//!   that serves that profile now needs the id.
//!
//! A profile leaves a follower that goes on where the gateway's own
//! verified answer no longer serves it, or a verified answer shows
//! another instance serving it while its own was not heard: that
//! profile's cards close, and the follower keeps its id for the rest. A
//! missing socket and a gateway that does not run are none of these.
//!
//! Where two instances both say they serve a profile, no card is opened
//! for it by either until a whole look finds one of them alone. A look
//! that missed a reply resolves no contest: it takes no profile from a
//! gateway that did not answer, and frees no profile that was contested.
//!
//! A follower that lives is never ended or renamed for its name. Where a
//! gateway found would be named as a follower already is (that follower
//! kept its id after its anchor profile left it), the newcomer waits: it
//! gets its follower once the name is free, and not before.
use super::{
    discovery::{Found, Seen},
    follower::{End, Shared},
    followers::Followers,
};
use agent_hud_agents::hermes::gateway::{Connection, GatewayId, Input, Profile};
use std::{
    collections::BTreeSet,
    sync::{Arc, PoisonError},
};

/// A follower that ended in this look: its reader, and why.
type Ended = Vec<(String, &'static str)>;

impl Shared {
    /// The instance this follower follows: the last that was ready.
    pub(super) fn bound(&self) -> Option<String> {
        self.model().gateway.instance().map(str::to_owned)
    }

    /// The instance it found at its home in place of that one.
    fn found(&self) -> Option<String> {
        let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        found.clone()
    }

    fn ask(&self, profile: &Profile) {
        *self.asked.lock().unwrap_or_else(PoisonError::into_inner) = profile.clone();
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.log.line(&format!("asks profile={}", profile.name()));
    }

    fn withhold(&self, contested: &BTreeSet<Profile>) {
        let mut model = self.model();
        if !model.stopped && model.gateway.withheld() != contested {
            self.take(&mut model, Input::Withheld(contested));
        }
    }
}

impl Followers {
    /// Makes the followers match what `seen` verified: first what became
    /// of each follower's gateway, then the gateways that have none. A
    /// follower that ends has ended before the next one is made.
    pub(super) fn settle(&self, seen: &Seen) {
        let mut set = self.set();
        if !self.active() {
            return;
        }
        let contested = self.contest(seen);
        let mut ended = Ended::new();
        let mut index = 0;
        while index < set.len() {
            match self.fate(&set[index], seen) {
                Some((why, word)) => {
                    let gone = set.remove(index);
                    gone.end(why);
                    ended.push((gone.reader.clone(), word));
                }
                None => index += 1,
            }
        }
        for found in seen.instances.values() {
            self.found(&mut set, found, &mut ended);
        }
        for follower in set.iter() {
            follower.withhold(&contested);
        }
        // A restart that kept its id is one gateway going on, and said by
        // its cards alone.
        for (reader, word) in ended {
            let goes_on = *word == *"restarted" && set.iter().any(|next| next.reader == reader);
            if !goes_on {
                self.log
                    .line(&format!("gateway gone id={reader} why={word}"));
            }
        }
    }

    /// The profiles two instances claim, as of `seen`, said when that
    /// changes. A whole look says which there are. One that missed a reply
    /// only adds what it saw: whoever did not answer may still claim.
    fn contest(&self, seen: &Seen) -> BTreeSet<Profile> {
        let mut last = self
            .contested
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut contested = seen.contested();
        if seen.missed {
            contested.extend(last.iter().cloned());
        }
        if *last != contested {
            let names: Vec<&str> = contested.iter().map(Profile::name).collect();
            self.log
                .line(&format!("contested profiles=[{}]", names.join(",")));
            *last = contested.clone();
        }
        contested
    }

    /// Says once that the gateway running as `instance` waits for the
    /// name `reader`.
    fn waits(&self, reader: &str, instance: &str) {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        if waiting.insert(instance.to_owned()) {
            let short: String = instance.chars().take(8).collect();
            self.log
                .line(&format!("gateway waits id={reader} instance={short}"));
        }
    }

    /// What `seen` means for one follower: the profiles that left it are
    /// retired, the home it asks is changed where that home is another
    /// gateway's, and what its own gateway says of itself now is taken.
    /// Some where the follower ends.
    fn fate(&self, follower: &Shared, seen: &Seen) -> Option<(End, &'static str)> {
        let asked = follower.asked();
        let mut model = follower.model();
        // One that was never ready follows nothing yet.
        let bound = model.gateway.instance()?.to_owned();
        let held = model.gateway.holds().clone();
        let at_asked = seen.at.get(&asked);
        let heard = seen.instances.get(&bound);
        let left = |profile: &&Profile| match heard {
            Some(found) => !found.instance.serves(profile),
            // Not heard: another instance serving it takes it, where the
            // look was whole. One that missed a reply may have missed this
            // gateway's, and takes nothing from it.
            None => !seen.missed && seen.sole(profile).is_some(),
        };
        if let (None, Some(other)) = (heard, at_asked) {
            // Its home answers as another instance and its own was heard
            // nowhere: a restart, once the follower has found it for
            // itself. Until then it is left as it is, and asked to look.
            let found = follower.found().as_ref() == Some(other);
            return found.then_some((End::Restarted, "restarted"));
        }
        let gone: BTreeSet<Profile> = held.iter().filter(left).cloned().collect();
        if !gone.is_empty() {
            let names: Vec<&str> = gone.iter().map(Profile::name).collect();
            let line = format!("retired profiles=[{}]", names.join(","));
            follower.log.line(&line);
            follower.take(&mut model, Input::Retired(&gone));
        }
        // The home it asks is its gateway's no more: another of them is.
        let other = heard.is_some() && at_asked.is_some_and(|at| *at != bound);
        let asks = match heard {
            _ if !gone.contains(&asked) && !other => asked,
            Some(found) => found.via.keys().next()?.clone(),
            None => match model.gateway.holds().iter().next() {
                Some(kept) => kept.clone(),
                None => return Some((End::Replaced, "emptied")),
            },
        };
        let moved = asks != follower.asked();
        if moved {
            follower.ask(&asks);
        }
        // What its gateway says of itself now, while it is the one that
        // is ready: the profiles it serves, and the home it is asked in.
        let (answer, home) = heard?.via.get(&asks)?;
        let grew = match model.gateway.connection() {
            Connection::Ready(ready) if ready.id == bound => {
                let now = &heard?.instance.served;
                moved || now.iter().any(|served| !ready.serves(&served.profile))
            }
            _ => false,
        };
        if grew {
            let root = &seen.root;
            follower.take(&mut model, Input::Identified { answer, home, root });
        }
        None
    }

    /// Makes the follower of a gateway that has none. One that a follower
    /// is about to find at the home it asks is left to that follower,
    /// which is asked to look now.
    fn found(&self, set: &mut Vec<Arc<Shared>>, found: &Found, ended: &mut Ended) {
        let instance = &found.instance;
        let follows = |follower: &Arc<Shared>| follower.bound().as_ref() == Some(&instance.id);
        if set.iter().any(follows) {
            return;
        }
        let asks = |follower: &&Arc<Shared>| found.via.contains_key(&follower.asked());
        if let Some(follower) = set.iter().find(asks) {
            return follower.stir();
        }
        let Some(anchor) = instance.anchor() else {
            return;
        };
        let id = GatewayId::new(self.machine.clone(), anchor.clone());
        let reader = id.to_string();
        if set.iter().any(|kept| kept.reader == reader) {
            // The name is a living follower's, which kept it after its
            // anchor profile left its gateway. That follower is not ended
            // and not renamed: this gateway waits until the name is free.
            return self.waits(&reader, &instance.id);
        }
        // The anchor's own home is asked, where it answered.
        let Some(asked) = [&anchor]
            .into_iter()
            .chain(found.via.keys())
            .find(|at| found.via.contains_key(at))
        else {
            return;
        };
        let born = found.via.get(asked).map(|(answer, _)| answer.clone());
        let restarted = |(was, word): &(String, &str)| *was == reader && *word == "restarted";
        if !ended.iter().any(restarted) {
            let (short, profiles) = (instance.id.chars().take(8), instance.served.len());
            let short: String = short.collect();
            let line = format!("gateway found id={reader} instance={short} profiles={profiles}");
            self.log.line(&line);
        }
        if self.make(set, id, asked.clone(), born).is_err() {
            self.log.line(&format!("gateway not followed id={reader}"));
        }
    }
}
