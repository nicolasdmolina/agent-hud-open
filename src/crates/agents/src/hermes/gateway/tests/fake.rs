//! A pretend gateway for the tests: it holds sessions and answers a poll's
//! steps the way Hermes does (`gateway/session_events.py`, `since`;
//! `gateway/session_active_list.py`, `_status`; `session.resume`). The
//! tests say what happens in Hermes, run a poll, and look only at the
//! cards and the effects that come out.
use super::super::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Stream {
    pub epoch: String,
    /// The events Hermes still keeps, oldest first, as a replay carries them.
    pub events: Vec<Value>,
    pub latest: u64,
    /// Everything up to here has been dropped (`_replay_evicted_through`).
    pub dropped: u64,
    /// The questions and approvals that wait (`PendingControls.pending`).
    pub prompts: Vec<Value>,
    /// The sets of questions that wait (`open_requests`).
    pub open: Vec<Value>,
    /// The newest turn start or end, which says whether a turn runs.
    pub running: bool,
    /// What the list says in place of what Hermes would work out.
    pub status: Option<String>,
}

impl Stream {
    /// `_status`: waiting exactly while a question or an approval waits.
    pub fn status(&self) -> String {
        match &self.status {
            Some(said) => said.clone(),
            None if !self.prompts.is_empty() => "waiting".into(),
            None if self.running => "working".into(),
            None => "idle".into(),
        }
    }
    /// `SessionEvents.since`.
    fn since(&self, epoch: Option<&str>, after: u64) -> Value {
        let missing = epoch != Some(&self.epoch) || after > self.latest || after < self.dropped;
        let events: Vec<&Value> = self
            .events
            .iter()
            .filter(|event| !missing && event["seq"].as_u64() > Some(after))
            .collect();
        json!({
            "events": events, "count": events.len(),
            "latest_seq": self.latest, "last_sequence": self.latest,
            "epoch": self.epoch, "replay_epoch": self.epoch,
            "truncated": missing, "snapshot_required": missing,
            "open_requests": self.open,
        })
    }
}

#[derive(Default)]
pub struct Hermes {
    pub streams: BTreeMap<(Profile, String), Stream>,
    /// Steps that fail when asked, each once.
    pub failing: Vec<Step>,
    /// Every step the last poll asked, in order.
    pub asked: Vec<Step>,
}

impl Hermes {
    pub fn stream(&mut self, profile: &Profile, session: &str) -> &mut Stream {
        self.streams
            .entry((profile.clone(), session.to_owned()))
            .or_insert_with(|| Stream {
                epoch: "epoch-1".into(),
                ..Stream::default()
            })
    }
    /// Publishes an event on a session's stream, as Hermes stamps one.
    pub fn publish(
        &mut self,
        profile: &Profile,
        session: &str,
        kind: &str,
        turn: u64,
        body: Value,
    ) {
        let stream = self.stream(profile, session);
        stream.latest += 1;
        match kind {
            "clarify.request" | "approval.request" => stream.prompts.push(body.clone()),
            "clarify.settled" | "approval.settled" => stream
                .prompts
                .retain(|prompt| prompt["prompt_id"] != body["prompt_id"]),
            "message.start" => stream.running = true,
            "message.complete" => stream.running = false,
            _ => {}
        }
        stream.events.push(json!({
            "type": kind, "session_id": session, "execution_generation": turn,
            "replay_epoch": stream.epoch, "seq": stream.latest, "payload": body,
        }));
    }
    /// Hermes drops the oldest events it keeps, up to and including `seq`.
    pub fn drop_through(&mut self, profile: &Profile, session: &str, seq: u64) {
        let stream = self.stream(profile, session);
        stream.dropped = seq;
        stream
            .events
            .retain(|event| event["seq"].as_u64() > Some(seq));
    }
    /// Hermes's replay cache dropped the session: its stream starts again
    /// under a new epoch, and what waited still waits.
    pub fn new_epoch(&mut self, profile: &Profile, session: &str, epoch: &str) {
        let stream = self.stream(profile, session);
        stream.epoch = epoch.to_owned();
        stream.events.clear();
        (stream.latest, stream.dropped) = (0, 0);
    }
    /// The result Hermes gives a step, or None where the call fails.
    pub fn answer(&mut self, step: &Step) -> Option<Value> {
        self.asked.push(step.clone());
        if let Some(index) = self.failing.iter().position(|failing| failing == step) {
            self.failing.remove(index);
            return None;
        }
        match step {
            Step::List(profile) => {
                let rows: Vec<Value> = self
                    .streams
                    .iter()
                    .filter(|((of, _), _)| of == profile)
                    .map(|((_, id), stream)| json!({"id": id, "status": stream.status()}))
                    .collect();
                Some(json!({ "sessions": rows }))
            }
            Step::Since { session, cursor } => {
                let stream = self.held(session)?;
                let epoch = cursor.as_ref().map(|cursor| cursor.epoch.as_str());
                Some(stream.since(epoch, cursor.as_ref().map_or(0, |cursor| cursor.seq)))
            }
            Step::Snapshot(session) => {
                let stream = self.held(session)?;
                Some(json!({
                    "session_id": session.id, "replay_epoch": stream.epoch,
                    "last_sequence": stream.latest, "subscription_id": "synthetic-sub",
                    "prompts": stream.prompts, "open_requests": stream.open,
                }))
            }
        }
    }
    /// A session's stream, where anything is known of it.
    fn held(&self, session: &SessionRef) -> Option<&Stream> {
        let stream = self
            .streams
            .get(&(session.profile.clone(), session.id.clone()))?;
        (!stream.epoch.is_empty()).then_some(stream)
    }
    /// How many snapshots the last poll took.
    pub fn snapshots(&self) -> usize {
        let snapshot = |step: &&Step| matches!(step, Step::Snapshot(_));
        self.asked.iter().filter(snapshot).count()
    }
}

/// Runs `poll` to its end against the pretend gateway, as the connection
/// will against the real one.
pub fn ask(poll: &mut Poll, hermes: &mut Hermes) {
    hermes.asked.clear();
    for _ in 0..64 {
        let steps = poll.steps();
        if steps.is_empty() {
            return;
        }
        for step in steps {
            match hermes.answer(&step) {
                Some(result) => poll.answer(&step, &result),
                None => poll.failed(&step),
            }
        }
    }
    panic!("the poll never ends: {:?}", poll.steps());
}

/// One whole poll: begun, asked, and given to the model.
pub fn poll(gateway: &mut Gateway, hermes: &mut Hermes, now_ms: i64) -> Vec<Effect> {
    let Some(mut poll) = gateway.poll() else {
        return Vec::new();
    };
    ask(&mut poll, hermes);
    gateway.apply(Input::Polled(poll), now_ms)
}

/// An `identify` answer for a gateway that is ready, serving `profiles`.
pub fn ready(instance: &str, profiles: &[&str]) -> Value {
    let served: Vec<Value> = profiles
        .iter()
        .map(|name| match *name {
            "default" => json!({"profile_id": "/synthetic", "home": "/synthetic"}),
            name => {
                let home = format!("/synthetic/profiles/{name}");
                json!({"profile_id": home, "home": home})
            }
        })
        .collect();
    json!({
        "runtime_protocol": 1, "state": "ready", "instance_id": instance,
        "authority_epoch": 1, "api_origin": "http://127.0.0.1:54321",
        "capabilities": [SESSION_AUTHORITY], "served_profiles": served,
    })
}

/// Makes the model ready for `instance`, serving the default profile and
/// one named `verify`.
pub fn identified(gateway: &mut Gateway, instance: &str) -> Vec<Effect> {
    let answer = ready(instance, &["default", "verify"]);
    gateway.apply(
        Input::Identified {
            answer: &answer,
            home: "/synthetic",
        },
        0,
    )
}

/// The gateway's ids of every card showing.
pub fn names(gateway: &Gateway) -> Vec<String> {
    gateway
        .waiting()
        .map(|pending| pending.ask.key().to_owned())
        .collect()
}
