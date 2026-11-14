//! What a look at the machine changes (`discovery.rs`). Only what the
//! look verified counts, so a look that reached no home, or only some,
//! leaves every follower and every card it did not hear of as they were.
//!
//! A follower ends when client hooks turn off, or when its home answers
//! as another instance, which the follower found for itself, and its own
//! instance was heard nowhere: the gateway restarted. Its cards close as
//! a restart, and the gateway that answers now gets a follower of its
//! own, its anchor picked again. Nothing else ends one: not a missing
//! socket, not a gateway that does not run, and never its name.
//!
//! A profile leaves a follower that goes on where the gateway's own
//! verified answer no longer serves it: that profile's cards close, and
//! the follower keeps its id for the rest. Nothing but an answer of that
//! same instance takes a profile from it. A gateway that was followed
//! and was not heard in a look keeps every claim it had.
//!
//! Where two gateways both claim a profile, no card is opened for it by
//! either. The claims are what each instance that answered says it
//! serves, and what each follower whose instance did not answer still
//! holds. A contest ends when a look hears every claimant and one of
//! them alone still claims the profile. A gateway that is gone is no
//! claimant: its follower lost its socket and found nobody listening at
//! its control socket. That follower stays, with its cards, and goes on
//! asking; a gateway that only stalls, or only lost its control socket,
//! is not gone and keeps its claims.
//!
//! A gateway that would be named as a living follower already is (that
//! follower kept its id after its anchor profile left it) is told apart
//! by its own instance (`GatewayId::apart`) and followed at once. The
//! living follower and its name are left as they are.
use super::{
    discovery::{Found, Seen},
    follower::{End, Shared},
    followers::Followers,
};
use agent_hud_agents::hermes::gateway::{
    anchor, served_homes, Connection, GatewayId, Input, Profile,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{atomic::Ordering, Arc, PoisonError},
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

    fn ask(&self, place: &Profile) {
        *self.asked.lock().unwrap_or_else(PoisonError::into_inner) = place.clone();
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.log.line(&format!("asks profile={}", place.name()));
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
        // Before any follower is made: one begins with what is contested.
        let contested = self.contest(&set, seen);
        for found in seen.instances.values() {
            self.found(&mut set, found, seen, &ended);
        }
        for follower in set.iter() {
            follower.withhold(&contested);
        }
        // A restart that kept its id is one gateway going on, and said by
        // its cards alone.
        for (reader, word) in ended {
            if !set.iter().any(|next| next.reader == reader) {
                self.log
                    .line(&format!("gateway gone id={reader} why={word}"));
            }
        }
    }

    /// The profiles two gateways claim, said when that changes. An
    /// instance that answered claims what its answer serves. A follower
    /// whose instance did not answer, and whose home did not answer as
    /// another, claims what it still holds: no socket there is not its
    /// word that it let go. A look that missed a reply, or did not hear a
    /// follower's instance, ends no contest there was. A follower whose
    /// gateway is gone (`Shared::gone`) is the one exception: a gateway
    /// that runs no more claims nothing, so a contest it was part of ends
    /// at the next whole look, with no one having to turn hooks off.
    fn contest(&self, set: &[Arc<Shared>], seen: &Seen) -> BTreeSet<Profile> {
        let mut claims: BTreeMap<Profile, usize> = BTreeMap::new();
        let mut claim = |profile: &Profile| *claims.entry(profile.clone()).or_default() += 1;
        let answered = seen.instances.values();
        let served = answered.flat_map(|found| found.instance.served.iter());
        served.for_each(|served| claim(&served.profile));
        let mut whole = !seen.missed;
        for follower in set {
            let model = follower.model();
            let heard = |bound: &str| seen.instances.contains_key(bound);
            if model.gateway.instance().is_none_or(heard) {
                continue;
            }
            // Its gateway is gone, as the follower found for itself: it
            // claims nothing, and it is no claimant left to hear.
            if follower.gone.load(Ordering::Acquire) {
                continue;
            }
            whole = false;
            if !seen.at.contains_key(&follower.asked()) {
                model.gateway.holds().iter().for_each(&mut claim);
            }
        }
        let twice = claims.into_iter().filter(|(_, claims)| *claims > 1);
        let mut contested: BTreeSet<Profile> = twice.map(|(profile, _)| profile).collect();
        let mut last = self
            .contested
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !whole {
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

    /// What `seen` means for one follower: the profiles its gateway's own
    /// answer serves no more are retired, the home it asks is changed
    /// where that home is its gateway's no more, and what its gateway
    /// says of itself now is taken. Some where the follower ends.
    fn fate(&self, follower: &Shared, seen: &Seen) -> Option<(End, &'static str)> {
        let asked = follower.asked();
        let mut model = follower.model();
        // One that was never ready follows nothing yet.
        let bound = model.gateway.instance()?.to_owned();
        let at_asked = seen.at.get(&asked);
        let Some(heard) = seen.instances.get(&bound) else {
            // Its instance was not heard, so nothing is taken from it. Its
            // home answering as another instance is a restart, once the
            // follower has found that for itself; until then it is left
            // as it is, and asked to look.
            let found = at_asked.is_some() && follower.found().as_ref() == at_asked;
            return found.then_some((End::Restarted, "restarted"));
        };
        let left = |profile: &&Profile| !heard.instance.serves(profile);
        let held = model.gateway.holds().clone();
        let gone: BTreeSet<Profile> = held.iter().filter(left).cloned().collect();
        if !gone.is_empty() {
            let names: Vec<&str> = gone.iter().map(Profile::name).collect();
            let line = format!("retired profiles=[{}]", names.join(","));
            follower.log.line(&line);
            follower.take(&mut model, Input::Retired(&gone));
        }
        // The home it asks is its gateway's no more: another of them is.
        let moved = gone.contains(&seen.named(&asked)) || at_asked.is_some_and(|at| *at != bound);
        let asks = match moved {
            true => heard.via.keys().next()?.clone(),
            false => asked,
        };
        if moved {
            follower.ask(&asks);
        }
        // What its gateway says of itself now, while it is the one that
        // is ready: the profiles it serves, and the home it is asked in.
        let (answer, home) = heard.via.get(&asks)?;
        let grew = match model.gateway.connection() {
            Connection::Ready(ready) if ready.id == bound => {
                let now = &heard.instance.served;
                moved || now.iter().any(|served| !ready.serves(&served.profile))
            }
            _ => false,
        };
        if grew {
            follower.take(&mut model, Input::Identified { answer, home });
        }
        None
    }

    /// Makes the follower of a gateway that has none. One that a follower
    /// is about to find at the home it asks is left to that follower,
    /// which is asked to look now.
    fn found(&self, set: &mut Vec<Arc<Shared>>, found: &Found, seen: &Seen, ended: &Ended) {
        let instance = &found.instance;
        let follows = |follower: &Arc<Shared>| follower.bound().as_ref() == Some(&instance.id);
        if set.iter().any(follows) {
            return;
        }
        let asks = |follower: &&Arc<Shared>| found.via.contains_key(&follower.asked());
        if let Some(follower) = set.iter().find(asks) {
            return follower.stir();
        }
        // Named by its anchor, which the machine's root picks among the
        // homes it serves. The root names the gateway and no session.
        let Some((place, (answer, _))) = found.via.iter().next() else {
            return;
        };
        let Some(anchor) = anchor(&seen.root, served_homes(answer)) else {
            return;
        };
        let taken = |id: &GatewayId| set.iter().any(|kept| kept.reader == id.to_string());
        let mut id = GatewayId::new(self.machine.clone(), anchor.clone());
        if taken(&id) {
            // The name is a living follower's, which kept it after its
            // anchor profile left its gateway. That follower is not ended
            // and not renamed: this gateway is told apart by its instance.
            id = GatewayId::apart(self.machine.clone(), anchor.clone(), &instance.id);
            if taken(&id) {
                return;
            }
        }
        // The anchor's own home is asked, where it answered.
        let asked = match found.via.contains_key(&anchor) {
            true => &anchor,
            false => place,
        };
        let born = found.via.get(asked).map(|(answer, _)| answer.clone());
        let reader = id.to_string();
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
