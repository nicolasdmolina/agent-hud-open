//! Which moments deserve a sound, decided once each: a session that starts
//! waiting on the user (`needs-you`) and a turn that ends (`finished`).
//! Steps and tool calls make none. Core only decides; the host plays the
//! named sound for each alert id it has not played yet.
//!
//! Several requests that newly wait in one session at the same look make one
//! `needs-you`, not one each: one ping per session per look. Every one of
//! them is decided then, so none sounds on its own later.
use super::*;

/// A request a session waits on: its id, its kind, and when it opened.
struct Request {
    id: String,
    kind: String,
    at: Option<i64>,
}
/// What a session was waiting on at the last look: its requests (none for a
/// wait its hooks or process report without a card).
struct Wait {
    requests: Vec<Request>,
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
/// How close in time a transcript's sighting of a request and a card must
/// open to be taken for the same call: a card has no call id of its own.
const SAME_CALL_MS: i64 = 10_000;
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

fn requests(session: &Value) -> Vec<Request> {
    session["liveFacts"]["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Request {
                id: r["id"].as_str()?.into(),
                kind: r["kind"].as_str()?.into(),
                at: r["at"].as_str().and_then(metering::date_ms),
            })
        })
        .collect()
}
/// A transcript's sighting of a call (`log:` id) and the card for that call
/// are one wait, in either order: one of them just left, exactly one is the
/// transcript's, they are of one kind, and they opened within
/// `SAME_CALL_MS`. Two sightings, two cards, or a card opened later are
/// different requests.
fn taken_over(previous: &Wait, now: &[Request], new: &Request) -> bool {
    previous.requests.iter().any(|old| {
        old.kind == new.kind
            && !now.iter().any(|r| r.id == old.id)
            && old.id.starts_with("log:") != new.id.starts_with("log:")
            && old
                .at
                .zip(new.at)
                .is_some_and(|(a, b)| (a - b).abs() <= SAME_CALL_MS)
    })
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
        settings: &Value,
        now: i64,
    ) -> Value {
        let mut alerts = Vec::new();
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
            if self.first(&id, now) {
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
            let now_requests = requests(session);
            if session["state"] != "waiting" && now_requests.is_empty() {
                continue;
            }
            let key = format!("{source}:{id}");
            let previous = self.waits.remove(&key);
            let mut told = None;
            for request in &now_requests {
                let identity = format!("needs-you:{source}:{id}:{}", request.id);
                if !self.first(&identity, now) {
                    continue;
                }
                // A wait already told without a card, or the transcript's
                // sighting of the same call, is the same wait once its card
                // comes. The first new request of the look names the alert.
                let same_wait = previous.as_ref().is_some_and(|p| {
                    p.requests.is_empty() || taken_over(p, &now_requests, request)
                });
                if !same_wait {
                    told.get_or_insert(identity);
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
