//! Replays one recording through the model the way the connection will:
//! every frame the gateway sent goes in, and every answer the recording
//! sent is compared with what the model would send for the same decision.
use super::super::*;
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

pub fn describe(effect: &Effect, now_ms: i64) -> String {
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
            let wait = pending.deadline_ms.map_or(" no deadline".into(), |at| {
                format!(" wait {}s", (at - now_ms) / 1000)
            });
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
        Effect::Close {
            id, why: closed, ..
        } => format!("close {} {}", key(id), why(closed)),
        Effect::Unanswered { pending, why } => {
            format!("unanswered {} {why:?}", key(&pending.id()))
        }
        Effect::List => "list".into(),
        Effect::Read { session, cursor } => format!(
            "read {} from {}",
            short(&session.id),
            cursor
                .as_ref()
                .map_or("the start".into(), |c| c.seq.to_string())
        ),
        Effect::Resume { session } => format!("resume {}", short(&session.id)),
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

struct Run {
    gateway: Gateway,
    trace: Vec<String>,
    now_ms: i64,
    /// The recorded answers that the model would have sent itself; only
    /// their replies are the model's to hear.
    ours: Vec<u64>,
    /// The last count of live sessions said, which is said again on a change.
    live: String,
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

impl Run {
    fn apply(&mut self, input: Input<'_>) {
        let listed = matches!(input, Input::Listed { .. });
        let effects = self.gateway.apply(input, self.now_ms);
        let now = self.now_ms;
        for effect in &effects {
            self.say(describe(effect, now));
        }
        if listed {
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
        let at = self
            .gateway
            .session(profile, params["session_id"].as_str().unwrap_or(""));
        let (asked, said, decision) = match method {
            "clarify.respond" => {
                let prompt = params["prompt_id"].as_str().unwrap();
                let text = params["answer"].as_str().unwrap();
                let id = at.request_id(prompt);
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
                    at.request_id(prompt),
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

/// Checks that the model's own encoder writes a recorded reading call, and
/// says whether a recorded replay started where the model's cursor is.
fn check_call(gateway: &Gateway, sent: &Value, scope: Scope, profile: &Profile) -> Option<String> {
    let params = &sent["params"];
    let at = gateway.session(profile, params["session_id"].as_str().unwrap_or(""));
    let mut line = None;
    let ours = match sent["method"].as_str().unwrap() {
        "session.active_list" => active_list(profile, scope),
        "session.resume" => resume(&at, scope),
        "session.events.since" => {
            let cursor = params["replay_epoch"].as_str().map(|epoch| Cursor {
                epoch: epoch.to_owned(),
                seq: params["last_sequence"].as_u64().unwrap(),
            });
            let from = cursor
                .as_ref()
                .map_or("the start".into(), |c| c.seq.to_string());
            line = Some(if cursor.as_ref() == gateway.cursor(&at) {
                format!("asked for events from {from}: where the model's cursor is")
            } else {
                format!("asked for events from {from}: not where the model's cursor is")
            });
            events_since(&at, cursor.as_ref(), scope)
        }
        // The spike closed its own sessions with `end_reason`; the model never does.
        "session.detach" if params.get("end_reason").is_none() => {
            detach(&at, params["subscription_id"].as_str().unwrap(), scope)
        }
        _ => return None,
    };
    assert_eq!(ours.frame(sent["id"].as_u64().unwrap()), *sent);
    line
}

pub fn replay(name: &str) -> Vec<String> {
    let doc = fixture(name);
    let mut run = Run {
        gateway: local(),
        trace: Vec::new(),
        now_ms: 0,
        ours: Vec::new(),
        live: String::new(),
    };
    for (client, frames, scope) in clients(&doc) {
        if client != "hud" {
            super::handshake::between_sockets(&doc, &mut |input, note| {
                if let Some(note) = note {
                    run.say(note);
                }
                run.apply(input);
            });
            run.say(format!("-- {client}"));
        } else {
            super::handshake::before_first_socket(&doc, &mut |input| run.apply(input));
        }
        let base = run.now_ms;
        let mut calls: BTreeMap<u64, Value> = BTreeMap::new();
        let mut asked_in: Option<SessionRef> = None;
        for entry in frames {
            let sent = &entry["frame"];
            run.now_ms = base + (entry["t"].as_f64().unwrap() * 1000.0) as i64;
            // The socket's own profile, or the one a host's socket names.
            let profile = match (scope, sent["params"]["profile"].as_str()) {
                (Scope::Host, named) => Profile::named(named.unwrap_or("")),
                _ => spike(),
            };
            if entry["dir"] == "out" {
                if let Some(id) = sent["id"].as_u64() {
                    calls.insert(id, sent.clone());
                    match sent["method"].as_str().unwrap() {
                        "clarify.respond" | "approval.respond" => {
                            run.compare(sent, scope, &profile)
                        }
                        _ => {
                            if let Some(line) = check_call(&run.gateway, sent, scope, &profile) {
                                run.say(line);
                            }
                        }
                    }
                } else {
                    run.compare_batch(sent, asked_in.as_ref().unwrap());
                }
                continue;
            }
            run.apply(Input::Tick);
            let Some(call) = sent["id"].as_u64().and_then(|id| calls.get(&id)).cloned() else {
                if let Frame::Request { params, .. } = frame(sent) {
                    let session = params["session_id"].as_str().unwrap();
                    asked_in = Some(run.gateway.session(&profile, session));
                }
                run.apply(Input::Frame {
                    profile: &profile,
                    frame: sent,
                });
                continue;
            };
            let profile = match (scope, call["params"]["profile"].as_str()) {
                (Scope::Host, named) => Profile::named(named.unwrap_or("")),
                _ => spike(),
            };
            let result = &sent["result"];
            let session = call["params"]["session_id"].as_str().unwrap_or("");
            match call["method"].as_str().unwrap() {
                "session.active_list" if result.is_object() => run.apply(Input::Listed {
                    profile: &profile,
                    result,
                }),
                "session.events.since" if result.is_object() => run.apply(Input::Replayed {
                    profile: &profile,
                    session,
                    result,
                }),
                "session.resume" if result.is_object() => run.apply(Input::Resumed {
                    profile: &profile,
                    result,
                }),
                "clarify.respond" | "approval.respond" => {
                    let prompt = call["params"]["prompt_id"].as_str().unwrap();
                    let request = run.gateway.session(&profile, session).request_id(prompt);
                    let outcome = match answered(sent) {
                        Answered::Resolved => "resolved".to_owned(),
                        Answered::AlreadyResolved => "already resolved".to_owned(),
                        Answered::Refused(refusal) => format!("refused {refusal:?}"),
                    };
                    if run.ours.contains(&sent["id"].as_u64().unwrap()) {
                        run.say(format!("reply {outcome}"));
                        run.apply(Input::Responded {
                            request: &request,
                            reply: sent,
                        });
                    } else {
                        run.say(format!(
                            "reply {outcome} (to an answer the model did not send)"
                        ));
                    }
                }
                _ => {}
            }
        }
    }
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
