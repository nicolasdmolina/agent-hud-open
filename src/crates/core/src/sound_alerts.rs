//! Which moments deserve a sound, decided once each: a session that starts
//! waiting on the user (`needs-you`) and a turn that ends (`finished`).
//! Steps and tool calls make none. Core only decides; the host plays the
//! named sound for each alert id it has not played yet.
//!
//! Several requests that newly wait in one session at the same look make one
//! `needs-you`, not one each: one ping per session per look. Every one of
//! them is decided then, so none sounds on its own later.
use super::*;

mod bare;
mod kept;
mod look;
mod requests;
mod wait;
pub(crate) use requests::Heard;
use wait::Wait;

pub(crate) struct Tracker {
    primed: bool,
    /// Each waiting session's wait, by `source:session`.
    waits: HashMap<String, Wait>,
    /// Every identity already decided, sounded or not, and when it was
    /// last among the inputs; one gone for a day is forgotten.
    decided: HashMap<String, i64>,
    /// Whether an identity was decided or forgotten since the file that
    /// keeps them across a relaunch was last told (`kept.rs`).
    changed: bool,
    kept: Option<kept::Kept>,
    /// The approved requests whose wait the last look saw end, by their
    /// `source:session` and id, until core is told (`take_ended`).
    ended: Vec<(String, String)>,
    revision: u64,
    items: VecDeque<Value>,
}
const KEPT_MS: i64 = 86_400_000;
/// Alerts of earlier looks kept beside the newest look's, as many as the
/// island keeps events; a look's own alerts are never dropped before a
/// snapshot carries them.
const KEPT_ITEMS: usize = 32;

/// The sounds a settings file without its own choice plays.
pub(crate) const NEEDS_YOU_SOUND: &str = "Glass";
pub(crate) const FINISHED_SOUND: &str = "Hero";

/// One alert before it is told: its kind, the session it belongs to, and the
/// identity it is played under once.
struct Alert {
    kind: &'static str,
    source: Value,
    session: Value,
    id: String,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            primed: false,
            waits: HashMap::new(),
            decided: HashMap::new(),
            changed: false,
            kept: None,
            ended: Vec::new(),
            revision: 0,
            items: VecDeque::new(),
        }
    }
    /// A tracker that starts with what the one before it decided and
    /// saved in `file`, less what is older than a day at `now`, and saves
    /// there what it decides. A request decided before a relaunch does
    /// not sound after it, whenever its card comes back.
    pub fn kept(file: std::path::PathBuf, now: i64) -> Self {
        let (kept, decided) = kept::Kept::load(file, now, KEPT_MS);
        Self {
            decided,
            kept: Some(kept),
            ..Self::new()
        }
    }
    /// Whether `id` is new, noting it as seen now either way.
    fn first(&mut self, id: &str, now: i64) -> bool {
        let new = self.decided.insert(id.into(), now).is_none();
        self.changed |= new;
        new
    }
    /// The alerts this look adds, from the island's completions and the
    /// sessions' waits; with `sound` off in `settings` it still notes what it
    /// saw, so turning sound on later tells nothing already there.
    pub fn update(
        &mut self,
        island: &Value,
        sessions: &[Value],
        heard: &Heard,
        settings: &Value,
        now: i64,
    ) -> Value {
        let mut alerts = Vec::new();
        // A subagent's turn ending is a step of another's turn: only a
        // top-level session's turn end is `finished`. The island's
        // completions come from these same rows, which name a child's parent
        // or, where the log names none, say it is one (`liveFacts.subagent`).
        let children: HashSet<(&str, &str)> = sessions
            .iter()
            .filter(|session| {
                !session["parentId"].is_null() || session["liveFacts"]["subagent"] == true
            })
            .filter_map(|session| Some((session["source"].as_str()?, session["id"].as_str()?)))
            .collect();
        for event in island["items"].as_array().into_iter().flatten() {
            if event["kind"] != "completion" {
                continue;
            }
            let (Some(source), Some(session), Some(turn)) = (
                event["source"].as_str(),
                event["sessionId"].as_str(),
                event["turnId"].as_str(),
            ) else {
                continue;
            };
            let id = format!("finished:{source}:{session}:{turn}");
            if self.first(&id, now) && !children.contains(&(source, session)) {
                alerts.push(Alert {
                    kind: "finished",
                    source: event["source"].clone(),
                    session: event["sessionId"].clone(),
                    id,
                });
            }
        }
        let mut waits = HashMap::new();
        for session in sessions {
            let (Some(source), Some(id)) = (session["source"].as_str(), session["id"].as_str())
            else {
                continue;
            };
            let key = format!("{source}:{id}");
            // What core says is approved now: allowed automatically, with
            // no card, and its answer confirmed delivered. Asked at every
            // look; nothing of an earlier look's answer is used.
            let no_names = Vec::new();
            let allowed = heard.approved.get(&key).unwrap_or(&no_names);
            let now_requests = requests::requests(session, source, id, heard);
            if session["state"] != "waiting" && now_requests.is_empty() {
                continue;
            }
            let previous = self.waits.remove(&key);
            let before = previous.as_ref();
            let mut told = None;
            // The requests its channel says the wait is for, and which of
            // them are approved: by their ids, and by nothing else.
            let named = heard.waits.get(&key).map_or(&[][..], Vec::as_slice);
            let is_approved = |request: &&String| allowed.contains(request);
            let approved: Vec<String> = named.iter().filter(is_approved).cloned().collect();
            let over = before.into_iter().flat_map(|p| &p.approved);
            let over = over.filter(|r| !approved.contains(r));
            self.ended.extend(over.map(|r| (key.clone(), r.clone())));
            // A wait with no card, for approved requests alone, is theirs:
            // it makes no sound. One that names no request always sounds.
            let silent =
                now_requests.is_empty() && !named.is_empty() && approved.len() == named.len();
            for request in &now_requests {
                // A transcript sighting that goes by an approved request's
                // name is that request, which showed no card: silent while
                // core says so, and not decided, so it sounds if core says
                // so no longer. A card always waits for the person.
                if !request.card && request.names.iter().any(|n| allowed.contains(n)) {
                    continue;
                }
                let names: Vec<String> = request
                    .names
                    .iter()
                    .map(|name| format!("needs-you:{source}:{id}:{name}"))
                    .collect();
                // New only if it goes by no name decided before; all its
                // names are decided now. Every name is checked before any is
                // marked: a card and its own sighting both name their call,
                // and that one call must not count as heard by itself.
                let new = names.iter().all(|name| !self.decided.contains_key(name));
                for name in &names {
                    self.first(name, now);
                }
                // A wait already told without a card is the same wait once
                // its card comes. The look's first new request names the alert.
                let told_bare = |p: &Wait| p.requests.is_empty() && !p.silent && p.may_be(request);
                if new && !before.is_some_and(told_bare) {
                    told.get_or_insert(names[0].clone());
                }
            }
            // The name a wait with no card had here goes on with the wait.
            let mut lent = before.and_then(|p| p.lent.clone());
            if silent {
                // The approved requests' wait: the one before it is over.
                self.over(lent.take(), now);
            } else if now_requests.is_empty() {
                let (identity, borrowed) = self.bare_name(session, source, id);
                lent = borrowed.then(|| identity.clone());
                if self.first(&identity, now) && before.is_none_or(|p| p.silent) {
                    told = Some(identity);
                }
            }
            if let Some(identity) = told {
                alerts.push(Alert {
                    kind: "needs-you",
                    source: session["source"].clone(),
                    session: session["id"].clone(),
                    id: identity,
                });
            }
            waits.insert(
                key,
                Wait {
                    requests: now_requests,
                    named: named.to_vec(),
                    approved,
                    silent,
                    lent,
                },
            );
        }
        // A session that waits no more: what its wait was for is over, and
        // so is the name it borrowed.
        for (key, wait) in std::mem::replace(&mut self.waits, waits) {
            let over = wait.approved.into_iter();
            self.ended
                .extend(over.map(|request| (key.clone(), request)));
            self.over(wait.lent, now);
        }
        let decided = self.decided.len();
        self.decided.retain(|_, at| now - *at < KEPT_MS);
        self.changed |= self.decided.len() != decided;
        if let Some(kept) = &mut self.kept {
            kept.keep(&self.decided, std::mem::take(&mut self.changed), now);
        }
        // What waits at the first look was there before the app: noted only.
        let primed = std::mem::replace(&mut self.primed, true);
        if settings["sound"] != true {
            self.items.clear();
        } else {
            let mut added = 0;
            for alert in alerts {
                if alert.kind == "needs-you" && !primed {
                    continue;
                }
                let (key, default) = match alert.kind {
                    "needs-you" => ("soundNeedsYou", NEEDS_YOU_SOUND),
                    _ => ("soundFinished", FINISHED_SOUND),
                };
                added += 1;
                self.revision = self.revision.wrapping_add(1);
                self.items.push_back(json!({"id":alert.id,"kind":alert.kind,"source":alert.source,"sessionId":alert.session,"sound":settings[key].as_str().unwrap_or(default),"at":iso(now)}));
            }
            while self.items.len() > KEPT_ITEMS + added {
                self.items.pop_front();
            }
        }
        json!({"revision":self.revision,"items":self.items})
    }
}

/// Whether a setting names a sound: a name, never a path. The host looks it
/// up among the system's own sounds.
pub(crate) fn names_a_sound(value: &Value) -> bool {
    value.as_str().is_some_and(|name| {
        !name.is_empty()
            && name.trim() == name
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || " -_".contains(c))
    })
}

#[cfg(test)]
mod tests;
