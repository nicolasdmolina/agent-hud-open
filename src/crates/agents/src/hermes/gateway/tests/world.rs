//! What a recording has shown of Hermes so far, as a pretend Hermes a poll
//! can be run against. It takes in every frame the gateway sent: events as
//! they happened, and what the gateway answered to the lists, replays and
//! snapshots the recording asked for. It holds a session's events only
//! where it has them without a gap, as Hermes's own stream does, and it
//! says a session waits by the same rule as Hermes: while a question or an
//! approval that was asked has not settled.
use super::super::*;
use super::fake::{Hermes, Stream};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct World {
    pub hermes: Hermes,
    /// The prompts seen to settle, so that an older request event read
    /// later (in a replay) does not make one wait again.
    settled: BTreeSet<String>,
    /// Where the recorded list said something else than this world works
    /// out by Hermes's rule. Empty at the end of every recording.
    pub differs: Vec<String>,
}

fn seq(event: &Value) -> u64 {
    event["seq"].as_u64().unwrap_or(0)
}

impl World {
    /// Hermes took an answer to this prompt. It publishes the settlement
    /// before it replies (`PendingControls.respond`), so the stream holds
    /// it by the time the reply is read, though the socket of the
    /// recording delivered the reply first: the settlement is taken in
    /// from further on in the recording.
    pub fn answered(&mut self, profile: &Profile, prompt: &str, later: &[Value]) {
        let settles = |entry: &&Value| {
            let params = &entry["frame"]["params"];
            let kind = params["type"].as_str().unwrap_or("");
            kind.ends_with(".settled") && params["payload"]["prompt_id"] == prompt
        };
        if let Some(entry) = later.iter().find(settles) {
            let upto = seq(&entry["frame"]["params"]);
            for entry in later {
                let params = &entry["frame"]["params"];
                if entry["dir"] == "in"
                    && entry["frame"]["method"] == "event"
                    && seq(params) <= upto
                {
                    self.event(profile, params);
                }
            }
        }
    }

    /// The gateway restarted: nothing of it is left.
    pub fn restart(&mut self) {
        self.hermes.streams.clear();
        self.settled.clear();
    }

    /// The profile a session is known under, else the one the socket says.
    fn profile(&self, session: &str, socket: &Profile) -> Profile {
        let known = |(profile, id): &&(Profile, String)| id == session && profile != socket;
        match self.hermes.streams.keys().find(known) {
            Some((profile, _))
                if !self
                    .hermes
                    .streams
                    .contains_key(&(socket.clone(), session.to_owned())) =>
            {
                profile.clone()
            }
            _ => socket.clone(),
        }
    }

    fn stream(&mut self, profile: &Profile, session: &str, epoch: &str) -> &mut Stream {
        let profile = self.profile(session, profile);
        let stream = self.hermes.stream(&profile, session);
        if stream.epoch != epoch {
            // Another stream: nothing of the one before is kept, and what
            // waited is not known until something says.
            *stream = Stream {
                epoch: epoch.to_owned(),
                ..Stream::default()
            };
        }
        stream
    }

    /// An event, as the socket delivered it or a replay carried it.
    pub fn event(&mut self, profile: &Profile, params: &Value) {
        let (Some(session), Some(epoch)) = (
            params["session_id"].as_str().filter(|id| !id.is_empty()),
            params["replay_epoch"].as_str(),
        ) else {
            return;
        };
        let at = seq(params);
        let payload = &params["payload"];
        let kind = params["type"].as_str().unwrap_or("");
        if let (true, Some(prompt)) = (kind.ends_with(".settled"), payload["prompt_id"].as_str()) {
            self.settled.insert(prompt.to_owned());
        }
        let over = payload["prompt_id"]
            .as_str()
            .is_some_and(|prompt| self.settled.contains(prompt));
        let stream = self.stream(profile, session, epoch);
        if at == 0 || stream.events.iter().any(|held| seq(held) == at) {
            return;
        }
        let place = stream.events.partition_point(|held| seq(held) < at);
        stream.events.insert(place, params.clone());
        // Only the run without a gap that ends at the newest event is held.
        let mut start = stream.events.len() - 1;
        while start > 0 && seq(&stream.events[start - 1]) + 1 == seq(&stream.events[start]) {
            start -= 1;
        }
        stream.events.drain(..start);
        stream.latest = stream.events.last().map_or(0, seq);
        stream.dropped = seq(&stream.events[0]) - 1;
        stream.status = None;
        match kind {
            "clarify.request" | "approval.request" => {
                let known = stream
                    .prompts
                    .iter()
                    .any(|prompt| prompt["prompt_id"] == payload["prompt_id"]);
                if !over && !known {
                    stream.prompts.push(payload.clone());
                }
            }
            "clarify.settled" | "approval.settled" => stream
                .prompts
                .retain(|prompt| prompt["prompt_id"] != payload["prompt_id"]),
            "request.cancel" => stream.open.retain(|open| open["id"] != payload["id"]),
            _ => {}
        }
        let turn = |held: &&Value| {
            matches!(
                held["type"].as_str(),
                Some("message.start" | "message.complete")
            )
        };
        if let Some(newest) = stream.events.iter().rev().find(turn) {
            stream.running = newest["type"] == "message.start";
        }
    }

    /// A set of questions the gateway asked its viewers.
    pub fn request(&mut self, profile: &Profile, frame: &Value) {
        let Some(session) = frame["params"]["session_id"].as_str() else {
            return;
        };
        let profile = self.profile(session, profile);
        let stream = self.hermes.stream(&profile, session);
        if !stream.open.iter().any(|open| open["id"] == frame["id"]) {
            let entry = serde_json::json!({"id": frame["id"], "method": frame["method"], "params": frame["params"]});
            stream.open.push(entry);
        }
    }

    /// The answer to `session.active_list`: which sessions live, and what
    /// each does until the next thing is heard of it.
    pub fn listed(&mut self, profile: &Profile, result: &Value) {
        let Some(rows) = result["sessions"].as_array() else {
            return;
        };
        let live: BTreeSet<&str> = rows.iter().filter_map(|row| row["id"].as_str()).collect();
        self.hermes
            .streams
            .retain(|(of, id), _| of != profile || live.contains(id.as_str()));
        for row in rows {
            let (Some(id), Some(said)) = (row["id"].as_str(), row["status"].as_str()) else {
                continue;
            };
            let known = self
                .hermes
                .streams
                .contains_key(&(profile.clone(), id.to_owned()));
            let stream = self.hermes.stream(profile, id);
            if !known {
                // Listed and nothing more: a read of it cannot be answered
                // until the recording shows its stream.
                stream.epoch = String::new();
            } else if stream.status.is_none()
                && stream.status() != said
                && !stream.events.is_empty()
            {
                self.differs.push(format!(
                    "{id}: listed {said}, worked out {}",
                    stream.status()
                ));
            }
            stream.status = Some(said.to_owned());
        }
    }

    /// The answer to `session.events.since`.
    pub fn replayed(&mut self, profile: &Profile, session: &str, result: &Value) {
        let Some(epoch) = result["replay_epoch"].as_str() else {
            return;
        };
        for params in result["events"].as_array().into_iter().flatten() {
            self.event(profile, params);
        }
        let latest = result["latest_seq"].as_u64().unwrap_or(0);
        let stream = self.stream(profile, session, epoch);
        if stream.events.is_empty() {
            (stream.latest, stream.dropped) = (latest, latest);
        }
        if let Some(open) = result["open_requests"].as_array() {
            stream.open = open.clone();
        }
    }

    /// The answer to `session.resume`: what waits, said outright.
    pub fn resumed(&mut self, profile: &Profile, result: &Value) {
        let (Some(session), Some(epoch)) = (
            result["session_id"].as_str(),
            result["replay_epoch"].as_str(),
        ) else {
            return;
        };
        let latest = result["last_sequence"].as_u64().unwrap_or(0);
        let stream = self.stream(profile, session, epoch);
        if stream.events.is_empty() {
            (stream.latest, stream.dropped) = (latest, latest);
        }
        stream.status = None;
        stream.running = result["running"] == true;
        if let Some(prompts) = result["prompts"].as_array() {
            stream.prompts = prompts.clone();
        }
        if let Some(open) = result["open_requests"].as_array() {
            stream.open = open.clone();
        }
    }
}
