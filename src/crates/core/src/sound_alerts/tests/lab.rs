//! A lab for the sound decision as the host hears it (Agent HUD Fork,
//! ticket 75): one session, `claude:s1`, whose wait is told one of four
//! ways, and each look's alerts put through a copy of the host's filter
//! (`host/src/system/sound.rs`, `fresh`: an id plays unless the snapshot
//! before carried it). Built from the verifiers' probes of rounds 1 and 2.
use super::*;

pub(super) const HOUR: i64 = 3_600_000;
/// The plain name a card of the session's request goes by.
pub(super) const CARD: &str = "needs-you:claude:s1:req-1";

/// How the session's wait is told.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    /// By its waiting flag alone.
    Bare,
    /// By a request its channel names, with no card.
    Named,
    /// By an ask its channel names, with no card.
    Ask,
    /// By the request's card, its channel naming it too.
    Card,
}
pub(super) const NO_CARD: [Kind; 3] = [Kind::Bare, Kind::Named, Kind::Ask];

pub(super) struct Lab {
    pub tracker: Tracker,
    played: HashSet<String>,
    pub now: i64,
}

impl Lab {
    pub fn new() -> Self {
        Self::with(started(), T)
    }
    /// A run of the app that keeps what it decides in `file`, from `now`.
    pub fn kept(file: &std::path::Path, now: i64) -> Self {
        Self::with(Tracker::kept(file.to_owned(), now), now)
    }
    fn with(tracker: Tracker, now: i64) -> Self {
        Self {
            tracker,
            played: HashSet::new(),
            now,
        }
    }
    /// One look `after` ms on, with `others` turn ends of other sessions:
    /// how many `needs-you` the host plays.
    pub fn see(
        &mut self,
        after: i64,
        row: Option<Value>,
        kind: Kind,
        others: usize,
        sound: bool,
    ) -> usize {
        self.now += after;
        let key = "claude:s1".to_owned();
        let waits = row.as_ref().is_some_and(|r| r["state"] == "waiting");
        let named: Vec<String> = match waits && kind != Kind::Bare {
            true => vec!["req-1".into()],
            false => vec![],
        };
        let asked = match kind {
            Kind::Ask => named.iter().map(|w| (w.clone(), w.clone())).collect(),
            _ => vec![],
        };
        let heard = Heard {
            waits: HashMap::from([(key.clone(), named)]),
            asks: HashMap::from([(key, asked)]),
            ..Heard::default()
        };
        let tag = self.now;
        let ends = (0..others).map(|n| json!({"id":format!("event:{tag}:{n}"),"kind":"completion","at":"2027-01-15T08:00:00Z","source":"codex","sessionId":format!("o-{tag}-{n}"),"turnId":format!("t-{tag}-{n}")}));
        let island = json!({"revision":1,"items":ends.collect::<Vec<_>>()});
        let rows: Vec<Value> = row.into_iter().collect();
        let settings = if sound { on() } else { off() };
        let out = self
            .tracker
            .update(&island, &rows, &heard, &settings, self.now);
        let mut seen = HashSet::new();
        let mut heard = 0;
        for item in out["items"].as_array().unwrap() {
            let id = item["id"].as_str().unwrap().to_owned();
            let fresh = seen.insert(id.clone()) && !self.played.contains(&id);
            heard += usize::from(fresh && item["kind"] == "needs-you");
        }
        self.played = seen;
        if sound {
            heard
        } else {
            0
        }
    }
    /// The session waits, told as `kind`.
    pub fn wait(&mut self, after: i64, kind: Kind) -> usize {
        self.see(after, Some(row(kind, None)), kind, 0, true)
    }
    /// The session is at work.
    pub fn run(&mut self, after: i64) -> usize {
        self.see(after, Some(running("s1")), Kind::Bare, 0, true)
    }
    /// The session is not listed.
    pub fn idle(&mut self, after: i64) -> usize {
        self.see(after, None, Kind::Bare, 0, true)
    }
    /// The counts of ended waits held, and the bytes of every name held.
    pub fn held(&self) -> (usize, usize) {
        let names = self.tracker.decided.keys().map(String::len).sum();
        (self.tracker.ends.len(), names)
    }
}

/// The session's row as it waits, told as `kind`, its last event at `since`.
pub(super) fn row(kind: Kind, since: Option<&str>) -> Value {
    let mut row = match kind {
        Kind::Card => waiting("s1", &[("req-1", "permission")]),
        _ => waiting("s1", &[]),
    };
    if let Some(since) = since {
        row["liveFacts"]["observedAt"] = json!(since);
    }
    row
}
