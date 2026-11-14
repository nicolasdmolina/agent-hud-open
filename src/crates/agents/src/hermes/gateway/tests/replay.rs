//! Replays one recording through the model. A recording is what one
//! socket saw of Hermes over time; the model reads Hermes by polling. So
//! each frame the gateway sent is first taken into a pretend Hermes
//! (`World`), which then holds what the real one held at that moment, and
//! a whole poll is run against it: the cards after every recorded frame.
//! Every answer the recording sent is compared with what the model would
//! send for the same decision.
use super::super::*;
use super::fake;
use super::world::World;
use super::{fixture, local, spike};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn short(id: &str) -> &str {
    &id[..id.len().min(14)]
}

fn key(id: &str) -> &str {
    let key = id.rsplit(':').next().unwrap_or(id);
    // A prompt id of an approval is 32 hex digits; eight tell them apart.
    if key.len() == 32 {
        &key[..8]
    } else {
        key
    }
}

fn why(closed: &Closed) -> String {
    format!("{closed:?}")
}

pub fn describe(effect: &Effect) -> String {
    match effect {
        Effect::Connection(connection) => match connection {
            Connection::Ready(instance) => format!(
                "connection ready {} epoch {} port {}",
                &instance.id[..8],
                instance.authority_epoch,
                instance.port
            ),
            other => format!("connection {other:?}"),
        },
        Effect::Open(pending) => {
            let wait = match pending.wait_ms {
                Some(wait) => format!(" waits {}s", wait / 1000),
                None => " says no wait".into(),
            };
            match &pending.ask {
                Ask::Question(q) => format!(
                    "open question {} gen {} {:?} [{}]{}{wait}",
                    q.prompt.id,
                    q.prompt.generation,
                    q.text,
                    q.choices.join("|"),
                    if q.multi_select { " multi" } else { "" },
                ),
                Ask::Approval(a) => format!(
                    "open approval {} gen {} {:?} [{}]{wait}",
                    key(&a.prompt.id),
                    a.prompt.generation,
                    a.command,
                    a.choices
                        .iter()
                        .map(|c| c.word())
                        .collect::<Vec<_>>()
                        .join("|"),
                ),
                Ask::Batch(b) => format!(
                    "open batch {} {}{wait}",
                    b.request,
                    b.questions
                        .iter()
                        .map(|q| format!(
                            "{}[{}]{}",
                            q.qid,
                            q.choices.join("|"),
                            if q.multi_select { " multi" } else { "" }
                        ))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
            }
        }
        Effect::Changed(pending) => format!("changed {}", key(&pending.id())),
        Effect::Marked(pending) => format!("out of reach {}", key(&pending.id())),
        Effect::Unmarked(pending) => format!("in reach {}", key(&pending.id())),
        Effect::Close {
            id, why: closed, ..
        } => format!("close {} {}", key(id), why(closed)),
        Effect::Unanswered { pending, why } => {
            format!("unanswered {} {why:?}", key(&pending.id()))
        }
        Effect::Poll => "poll now".into(),
    }
}

/// The decision a person would have made on the card to send what the
/// recording sent as the answer to a question.
fn answer_decision(key: &str, text: &str, choices: &[String], multi_select: bool) -> Value {
    let list: Option<Vec<String>> = serde_json::from_str(text).ok().filter(|_| multi_select);
    let answer = match list {
        Some(selected) => json!({"selected": selected}),
        None if choices.iter().any(|choice| choice == text) => json!({"selected": [text]}),
        None => json!({"selected": [], "custom": text}),
    };
    json!({"kind": "answer", "answers": {key: answer}})
}

fn choice_decision(word: &str) -> Value {
    match word {
        "once" => json!({"kind": "allow"}),
        "deny" => json!({"kind": "deny"}),
        "always" => json!({"kind": "always"}),
        "session" => json!({"kind": "always", "suggestion": {"type": "scope", "scope": "session"}}),
        other => json!({"kind": other}),
    }
}

/// Whether two frames say the same, reading an answer that is a JSON list
/// in a string as the list.
fn same(ours: &Value, theirs: &Value) -> bool {
    let list = |frame: &Value| -> Option<Vec<String>> {
        serde_json::from_str(frame["params"]["answer"].as_str()?).ok()
    };
    ours == theirs || {
        let (mut a, mut b) = (ours.clone(), theirs.clone());
        a["params"]["answer"] = Value::Null;
        b["params"]["answer"] = Value::Null;
        a == b && list(ours).is_some() && list(ours) == list(theirs)
    }
}

struct Replay {
    gateway: Gateway,
    world: World,
    trace: Vec<String>,
    now_ms: i64,
    /// The recorded answers that the model would have sent itself; only
    /// their replies are the model's to hear.
    ours: Vec<u64>,
    /// The last count of live sessions said, which is said again on a change.
    live: String,
    /// The last instance of the gateway that was ready.
    instance: Option<String>,
}

/// Where the model's frame differs from the recorded one.
fn differs(ours: &Value, theirs: &Value) -> String {
    ours["params"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, value)| theirs["params"][key.as_str()] != **value)
        .map(|(key, value)| format!("{key} {value}, not {}", theirs["params"][key.as_str()]))
        .collect::<Vec<_>>()
        .join("; ")
}

impl Replay {
    fn apply(&mut self, input: Input<'_>) {
        let effects = self.gateway.apply(input, self.now_ms);
        // Another instance is another Hermes: nothing of the old one is left.
        if let Some(now) = self.instance() {
            if self.instance.as_ref().is_some_and(|before| *before != now) {
                self.world.restart();
            }
            self.instance = Some(now);
        }
        for effect in &effects {
            self.say(describe(effect));
        }
    }

    fn instance(&self) -> Option<String> {
        match self.gateway.connection() {
            Connection::Ready(instance) => Some(instance.id.clone()),
            _ => None,
        }
    }

    /// One whole poll of Hermes as the recording has shown it so far, and
    /// the cards and sessions after it.
    fn poll(&mut self) {
        let Some(mut poll) = self.gateway.poll() else {
            return;
        };
        fake::ask(&mut poll, &mut self.world.hermes);
        for step in self.world.hermes.asked.clone() {
            if let Step::Snapshot(session) = step {
                self.say(format!("snapshot {}", short(session.id())));
            }
        }
        self.apply(Input::Polled(poll));
        let statuses = self.gateway.statuses();
        let count = |status| statuses.iter().filter(|(_, s)| *s == status).count();
        let live = format!(
            "live sessions: {} ({} working, {} waiting)",
            statuses.len(),
            count(Status::Working),
            count(Status::Waiting)
        );
        if live != self.live {
            self.live = live.clone();
            self.say(live);
        }
    }

    /// Adds a line, counting one that repeats the line before it.
    fn say(&mut self, line: String) {
        match self.trace.last_mut() {
            Some(last) if *last == line => *last = format!("{line} (x2)"),
            Some(last)
                if last
                    .strip_suffix(')')
                    .and_then(|l| l.rsplit_once(" (x"))
                    .is_some_and(|(l, _)| l == line) =>
            {
                let times: u32 = last
                    .rsplit_once(" (x")
                    .unwrap()
                    .1
                    .trim_end_matches(')')
                    .parse()
                    .unwrap();
                *last = format!("{line} (x{})", times + 1);
            }
            _ => self.trace.push(line),
        }
    }

    /// What the model would send for the recorded answer `sent`.
    fn compare(&mut self, sent: &Value, scope: Scope, profile: &Profile) {
        let params = &sent["params"];
        let method = sent["method"].as_str().unwrap_or("result");
        // Named only where the answer names its session.
        let named = self
            .gateway
            .session(profile, params["session_id"].as_str().unwrap_or(""));
        let at = || named.clone().expect("the answer names its session");
        let (asked, said, decision) = match method {
            "clarify.respond" => {
                let prompt = params["prompt_id"].as_str().unwrap();
                let text = params["answer"].as_str().unwrap();
                let id = at().request_id(prompt);
                let decision = match self.gateway.pending(&id).map(|p| &p.ask) {
                    Some(Ask::Question(q)) => {
                        answer_decision(prompt, text, &q.choices, q.multi_select)
                    }
                    _ => answer_decision(prompt, text, &[], false),
                };
                (id, format!("answer {text:?}"), decision)
            }
            _ => {
                let prompt = params["prompt_id"].as_str().unwrap();
                let word = params["choice"].as_str().unwrap();
                (
                    at().request_id(prompt),
                    format!("choice {word}"),
                    choice_decision(word),
                )
            }
        };
        let line = match self.gateway.pending(&asked) {
            None => format!("sent {method} {said}: no card for it"),
            Some(pending) => match pending.reply(&decision, scope) {
                Ok(Some(Reply::Call(call))) => {
                    let ours = call.frame(sent["id"].as_u64().unwrap());
                    if same(&ours, sent) {
                        self.ours.push(sent["id"].as_u64().unwrap());
                        self.apply(Input::Sent { request: &asked });
                        format!("sent {method} {said}: as the model encodes it")
                    } else {
                        format!(
                            "sent {method} {said}: the model would send {}",
                            differs(&ours, sent)
                        )
                    }
                }
                Ok(_) => format!("sent {method} {said}: the model sends nothing"),
                Err(error) => format!("sent {method} {said}: the model refuses ({error})"),
            },
        };
        self.say(line);
    }

    /// The same for the recorded result frame that answers a set of questions.
    fn compare_batch(&mut self, sent: &Value, session: &SessionRef) {
        let request = sent["id"].as_str().unwrap();
        let id = session.request_id(request);
        let line = match self.gateway.pending(&id) {
            None => format!("sent result {request}: no card for it"),
            Some(pending) => {
                let Ask::Batch(batch) = &pending.ask else {
                    panic!("{request} is not a set of questions");
                };
                let mut answers = serde_json::Map::new();
                for q in &batch.questions {
                    let Some(text) = sent["result"]["answers"][&q.qid].as_str() else {
                        continue;
                    };
                    let one = answer_decision(&q.qid, text, &q.choices, false);
                    answers.insert(q.qid.clone(), one["answers"][&q.qid].clone());
                }
                let decision = json!({"kind": "answer", "answers": answers});
                match pending.reply(&decision, Scope::Profile) {
                    Ok(Some(Reply::Result(ours))) if ours == *sent => {
                        self.apply(Input::Sent { request: &id });
                        format!("sent result {request}: as the model encodes it")
                    }
                    other => format!("sent result {request}: the model gives {other:?}"),
                }
            }
        };
        self.say(line);
    }
}

/// The frames of one client of a recording, with what its socket covers.
fn clients(doc: &Value) -> Vec<(&String, &Vec<Value>, Scope)> {
    doc["clients"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, frames)| {
            let scope = if name == "hud-host" {
                Scope::Host
            } else {
                Scope::Profile
            };
            (name, frames.as_array().unwrap(), scope)
        })
        .collect()
}

/// Checks that the model's own encoder writes a recorded reading call.
fn check_call(gateway: &Gateway, sent: &Value, scope: Scope, profile: &Profile) {
    let params = &sent["params"];
    // Named only where the call names its session.
    let named = gateway.session(profile, params["session_id"].as_str().unwrap_or(""));
    let at = || named.clone().expect("the call names its session");
    let ours = match sent["method"].as_str().unwrap() {
        "session.active_list" => Step::List(profile.clone()).command(scope),
        "session.resume" => Step::Snapshot(at()).command(scope),
        "session.events.since" => {
            let cursor = params["replay_epoch"].as_str().map(|epoch| Cursor {
                epoch: epoch.to_owned(),
                seq: params["last_sequence"].as_u64().unwrap(),
            });
            let session = at();
            Step::Since { session, cursor }.command(scope)
        }
        // The spike closed its own sessions with `end_reason`; the model never does.
        "session.detach" if params.get("end_reason").is_none() => {
            detach(&at(), params["subscription_id"].as_str().unwrap(), scope)
        }
        _ => return,
    };
    assert_eq!(ours.frame(sent["id"].as_u64().unwrap()), *sent);
}

/// Checks the pretend Hermes against the real one: asked the recorded
/// replay again, it answers what Hermes answered.
fn check_replay(world: &mut World, at: &SessionRef, call: &Value, result: &Value) {
    let cursor = call["params"]["replay_epoch"].as_str().map(|epoch| Cursor {
        epoch: epoch.to_owned(),
        seq: call["params"]["last_sequence"].as_u64().unwrap(),
    });
    let session = at.clone();
    let ours = world
        .hermes
        .answer(&Step::Since { session, cursor })
        .unwrap();
    let seqs = |reply: &Value| -> Vec<u64> {
        let events = reply["events"].as_array().unwrap();
        events
            .iter()
            .map(|event| event["seq"].as_u64().unwrap())
            .collect()
    };
    for key in ["snapshot_required", "truncated", "replay_epoch"] {
        assert_eq!(ours[key], result[key], "{key} of {call}");
    }
    assert_eq!(seqs(&ours), seqs(result), "{call}");
    // A refused read tells where the stream is by then, which may be past
    // what the recording had shown of it.
    if result["truncated"] == false {
        assert_eq!(ours["latest_seq"], result["latest_seq"], "{call}");
    }
}

pub fn replay(name: &str) -> Vec<String> {
    let doc = fixture(name);
    let mut run = Replay {
        gateway: local(),
        world: World::default(),
        trace: Vec::new(),
        now_ms: 0,
        ours: Vec::new(),
        live: String::new(),
        instance: None,
    };
    // The gateway as it last described itself while ready: the recorded
    // answer, or for a recording that kept none, the one the same
    // throwaway gateway gave in `gateway-self-description`.
    let mut described = super::handshake::usual();
    for (client, frames, scope) in clients(&doc) {
        if client != "hud" {
            super::handshake::between_sockets(&doc, &mut |input, note| {
                if let Some(note) = note {
                    run.say(note);
                }
                if let Input::Identified { answer, .. } = &input {
                    described = (*answer).clone();
                }
                run.apply(input);
            });
            run.say(format!("-- {client}"));
        } else {
            super::handshake::before_first_socket(&doc, &mut |input| {
                if let Input::Identified { answer, .. } = &input {
                    described = (*answer).clone();
                }
                run.apply(input)
            });
        }
        // A socket opens only on a gateway that has just said it is ready.
        let (answer, home) = (&described, super::handshake::HOME);
        match run.gateway.connection() {
            Connection::Ready(_) => {}
            Connection::Disconnected => {
                run.gateway
                    .apply(Input::Identified { answer, home }, run.now_ms);
                run.instance = run.instance();
            }
            _ => run.apply(Input::Identified { answer, home }),
        }
        let base = run.now_ms;
        let mut calls: BTreeMap<u64, Value> = BTreeMap::new();
        let mut asked_in: Option<SessionRef> = None;
        for (index, entry) in frames.iter().enumerate() {
            let sent = &entry["frame"];
            run.now_ms = base + (entry["t"].as_f64().unwrap() * 1000.0) as i64;
            // The socket's own profile, or the one a host's socket names.
            let profile = match (scope, sent["params"]["profile"].as_str()) {
                (Scope::Host, named) => Profile::named(named.unwrap_or("")).unwrap(),
                _ => spike(),
            };
            if entry["dir"] == "out" {
                if let Some(id) = sent["id"].as_u64() {
                    calls.insert(id, sent.clone());
                    match sent["method"].as_str().unwrap() {
                        "clarify.respond" | "approval.respond" => {
                            run.compare(sent, scope, &profile)
                        }
                        _ => check_call(&run.gateway, sent, scope, &profile),
                    }
                } else {
                    run.compare_batch(sent, asked_in.as_ref().unwrap());
                }
                continue;
            }
            let Some(call) = sent["id"].as_u64().and_then(|id| calls.get(&id)).cloned() else {
                match frame(sent) {
                    Frame::Request { params, .. } => {
                        let session = params["session_id"].as_str().unwrap();
                        asked_in = Some(run.gateway.session(&profile, session).unwrap());
                        run.world.request(&profile, sent);
                    }
                    Frame::Event(_) => run.world.event(&profile, &sent["params"]),
                    _ => {}
                }
                run.poll();
                continue;
            };
            let profile = match (scope, call["params"]["profile"].as_str()) {
                (Scope::Host, named) => Profile::named(named.unwrap_or("")).unwrap(),
                _ => spike(),
            };
            let result = &sent["result"];
            let session = call["params"]["session_id"].as_str().unwrap_or("");
            match call["method"].as_str().unwrap() {
                "session.active_list" if result.is_object() => run.world.listed(&profile, result),
                "session.events.since" if result.is_object() => {
                    run.world.replayed(&profile, session, result);
                    let at = run.gateway.session(&profile, session).unwrap();
                    check_replay(&mut run.world, &at, &call, result);
                }
                "session.resume" if result.is_object() => run.world.resumed(&profile, result),
                "clarify.respond" | "approval.respond" => {
                    let prompt = call["params"]["prompt_id"].as_str().unwrap();
                    let request = run
                        .gateway
                        .session(&profile, session)
                        .unwrap()
                        .request_id(prompt);
                    let outcome = match answered(sent) {
                        Answered::Resolved => "resolved".to_owned(),
                        Answered::AlreadyResolved => "already resolved".to_owned(),
                        Answered::Refused(refusal) => format!("refused {refusal:?}"),
                    };
                    if matches!(answered(sent), Answered::Resolved) {
                        run.world.answered(&profile, prompt, &frames[index..]);
                    }
                    if run.ours.contains(&sent["id"].as_u64().unwrap()) {
                        run.say(format!("reply {outcome}"));
                        let reply = sent;
                        run.apply(Input::Responded {
                            request: &request,
                            reply,
                        });
                    } else {
                        run.say(format!(
                            "reply {outcome} (to an answer the model did not send)"
                        ));
                    }
                }
                _ => {}
            }
            run.poll();
        }
    }
    assert_eq!(run.world.differs, [] as [String; 0], "{name}");
    let left: Vec<String> = run
        .gateway
        .waiting()
        .map(|p| key(&p.id()).to_owned())
        .collect();
    run.say(if left.is_empty() {
        "nothing left waiting".to_owned()
    } else {
        format!("left waiting: {}", left.join(" "))
    });
    run.trace
}

#[test]
fn every_recorded_scenario_gives_the_expected_cards_and_state() {
    let update = std::env::var_os("HERMES_TRACE").is_some();
    let mut wrong = Vec::new();
    for (name, expected) in super::scenarios::SCENARIOS {
        let trace = replay(name);
        if update {
            println!("    (\n        {name:?},\n        &[");
            for line in &trace {
                println!("            {line:?},");
            }
            println!("        ],\n    ),");
        } else if trace
            != expected
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
        {
            wrong.push(format!("{name}:\n  {}", trace.join("\n  ")));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
