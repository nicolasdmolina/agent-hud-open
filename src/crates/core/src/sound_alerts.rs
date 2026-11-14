//! Which moments deserve a sound, decided once each: a session that starts
//! waiting on the user (`needs-you`) and a turn that ends (`finished`).
//! Steps and tool calls make none. Core only decides; the host plays the
//! named sound for each alert id it has not played yet.
//!
//! Several requests that newly wait in one session at the same look make one
//! `needs-you`, not one each: one ping per session per look. Every one of
//! them is decided then, so none sounds on its own later.
//!
//! A waiting session whose row its transcript now gives, in place of one
//! only its cards or hooks made, is the same wait: each card that already
//! sounded, still waits and names no call stands for one new transcript
//! sighting of its own kind, which makes no sound. A card a call names is
//! that call only. A sighting no such card can stand for sounds.
use super::*;

mod requests;
pub(crate) use requests::Heard;
use requests::Request;

/// What a session was waiting on at the last look: its requests (none for a
/// wait its hooks or process report without a card), and whether its row
/// came from its transcript.
struct Wait {
    requests: Vec<Request>,
    transcript: bool,
}
pub(crate) struct Tracker {
    primed: bool,
    /// Each waiting session's wait, by `source:session`.
    waits: HashMap<String, Wait>,
    /// Every identity already decided, sounded or not, and when it was
    /// last among the inputs; one gone for a day is forgotten.
    decided: HashMap<String, i64>,
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
            revision: 0,
            items: VecDeque::new(),
        }
    }
    /// Whether `id` is new, noting it as seen now either way.
    fn first(&mut self, id: &str, now: i64) -> bool {
        self.decided.insert(id.into(), now).is_none()
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
            let now_requests = requests::requests(session, source, id, heard);
            if session["state"] != "waiting" && now_requests.is_empty() {
                continue;
            }
            let key = format!("{source}:{id}");
            let previous = self.waits.remove(&key);
            let transcript = !session["logPath"].is_null();
            // A row its transcript now gives, in place of one only its cards
            // or hooks made, shows what those cards already told: the
            // transcript's sightings of them, which it may not pair with
            // them one to one. Each card told before that still waits, and
            // that no call names (its `callId`, or the one possible pairing,
            // already makes it that call), can stand for one new sighting
            // of its own kind (`absorbs`); a sighting none can stand for is a
            // request of its own.
            let mut absorbs: HashMap<&str, usize> = HashMap::new();
            if let Some(p) = previous.as_ref().filter(|p| transcript && !p.transcript) {
                for heard in &p.requests {
                    let card = !heard.names[0].starts_with("log:");
                    let unnamed = now_requests
                        .iter()
                        .find(|r| r.names.contains(&heard.names[0]))
                        .is_some_and(|r| {
                            r.names
                                .iter()
                                .all(|n| !n.starts_with("call:") && !n.starts_with("log:"))
                        });
                    if card && unnamed {
                        *absorbs.entry(heard.kind.as_str()).or_default() += 1;
                    }
                }
            }
            let mut told = None;
            for request in &now_requests {
                let names: Vec<String> = request
                    .names
                    .iter()
                    .map(|name| format!("needs-you:{source}:{id}:{name}"))
                    .collect();
                // A new sighting a card told before stands for is decided
                // without a sound; one none can stand for sounds.
                let undecided = names.iter().all(|name| !self.decided.contains_key(name));
                if request.names[0].starts_with("log:") && undecided {
                    if let Some(left) = absorbs.get_mut(request.kind.as_str()).filter(|n| **n > 0) {
                        *left -= 1;
                        for name in &names {
                            self.first(name, now);
                        }
                        continue;
                    }
                }
                // New only if it goes by no name decided before; all its
                // names are decided now.
                let mut new = true;
                for name in &names {
                    new &= self.first(name, now);
                }
                // A wait already told without a card is the same wait once
                // its card comes. The look's first new request names the alert.
                if new && !previous.as_ref().is_some_and(|p| p.requests.is_empty()) {
                    told.get_or_insert(names[0].clone());
                }
            }
            if now_requests.is_empty() {
                let facts = &session["liveFacts"];
                let since = [&facts["attention"]["at"], &facts["observedAt"]]
                    .into_iter()
                    .find_map(Value::as_str)
                    .or(session["updatedAt"].as_str())
                    .unwrap_or("");
                let identity = format!("needs-you:{source}:{id}:{since}");
                if self.first(&identity, now) && previous.is_none() {
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
                    transcript,
                },
            );
        }
        self.waits = waits;
        self.decided.retain(|_, at| now - *at < KEPT_MS);
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

#[cfg(test)]
mod tests;
