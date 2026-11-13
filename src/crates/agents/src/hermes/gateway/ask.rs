//! What a Hermes session waits on a person for. Three things ask, and each
//! is its own type so that each takes only its own answer: a question
//! (`clarify.request`), an approval (`approval.request`), and a set of
//! questions sent as a request of the gateway's own (`clarify`, when the
//! turn's submitter declared `clarify_batch`, as the terminal UI does).
use super::identity::SessionRef;
use serde_json::Value;

/// One request waiting on the person, and where it waits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub session: SessionRef,
    pub ask: Ask,
    /// When Hermes stops waiting, where it said how long it waits. A set of
    /// questions never says, and a profile can turn its timeout off. The
    /// time runs from when Agent HUD first saw the request, so it is never
    /// earlier than Hermes's own.
    pub deadline_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Question(Question),
    Approval(Approval),
    Batch(Batch),
}

/// What an answer to a question or an approval must name: the prompt, and
/// the turn it was asked in. An answer for an earlier turn is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub prompt: Prompt,
    pub text: String,
    /// As offered, the first one marked ` (Recommended)` by the gateway.
    /// None offered is a question answered in the person's own words.
    pub choices: Vec<String>,
    pub multi_select: bool,
}

/// Hermes's four answers to an approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Choice {
    Once,
    Session,
    /// Writes a lasting rule into the profile's `config.yaml`.
    Always,
    Deny,
}

impl Choice {
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "once" => Self::Once,
            "session" => Self::Session,
            "always" => Self::Always,
            "deny" => Self::Deny,
            _ => return None,
        })
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Session => "session",
            Self::Always => "always",
            Self::Deny => "deny",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub prompt: Prompt,
    pub command: String,
    pub description: String,
    /// Only what Hermes offers for this request, in its order; a protected
    /// file or an editor's edit offers once and deny alone.
    pub choices: Vec<Choice>,
}

/// A set of questions asked at once. It has no prompt id, no turn and no
/// deadline: its answer is a result frame carrying the request's own id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    /// The gateway's request id (`srq-…`).
    pub request: String,
    pub questions: Vec<BatchQuestion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchQuestion {
    pub qid: String,
    pub text: String,
    pub choices: Vec<String>,
    pub multi_select: bool,
    /// What another viewer already answered, where a re-read says so.
    pub locked: Option<Locked>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Locked {
    Answered(String),
    Skipped,
}

impl Ask {
    /// The gateway's own id for it: a prompt id or a request id.
    pub fn key(&self) -> &str {
        match self {
            Self::Question(question) => &question.prompt.id,
            Self::Approval(approval) => &approval.prompt.id,
            Self::Batch(batch) => &batch.request,
        }
    }
    /// The turn it was asked in, where it says.
    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::Question(question) => Some(question.prompt.generation),
            Self::Approval(approval) => Some(approval.prompt.generation),
            Self::Batch(_) => None,
        }
    }
    /// A question or an approval, from the payload of its request event or
    /// its entry in a snapshot's `prompts`, with how long Hermes waits.
    pub fn prompt(payload: &Value) -> Option<(Self, Option<i64>)> {
        let prompt = Prompt {
            id: crate::text(&payload["prompt_id"])?.to_owned(),
            generation: payload["execution_generation"].as_u64()?,
        };
        let ask = match payload["kind"].as_str()? {
            "clarify" => Self::Question(Question {
                prompt,
                text: crate::text(&payload["question"])?.to_owned(),
                choices: words(&payload["choices"]),
                multi_select: payload["multi_select"] == true,
            }),
            "approval" => Self::Approval(Approval {
                prompt,
                command: payload["command"].as_str().unwrap_or("").to_owned(),
                description: payload["description"].as_str().unwrap_or("").to_owned(),
                choices: words(&payload["choices"])
                    .iter()
                    .filter_map(|word| Choice::parse(word))
                    .collect(),
            }),
            _ => return None,
        };
        let wait_ms = payload["timeout_s"]
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| (seconds * 1000.0) as i64);
        Some((ask, wait_ms))
    }
    /// A set of questions, from the gateway's `clarify` request or its
    /// entry in `open_requests`. Its other requests (a password, a secret)
    /// are never Agent HUD's to show.
    pub fn batch(id: &str, method: &str, params: &Value) -> Option<Self> {
        if method != "clarify" || id.is_empty() {
            return None;
        }
        let questions: Vec<BatchQuestion> = params["questions"]
            .as_array()?
            .iter()
            .filter_map(|question| {
                let qid = crate::text(&question["qid"])?;
                Some(BatchQuestion {
                    qid: qid.to_owned(),
                    text: crate::text(&question["question"])?.to_owned(),
                    choices: words(&question["choices"]),
                    multi_select: question["multi_select"] == true,
                    locked: match params["answers"].get(qid) {
                        Some(Value::String(answer)) => Some(Locked::Answered(answer.clone())),
                        Some(Value::Null) => Some(Locked::Skipped),
                        _ => None,
                    },
                })
            })
            .collect();
        (!questions.is_empty()).then_some(Self::Batch(Batch {
            request: id.to_owned(),
            questions,
        }))
    }
}

impl Pending {
    /// The id its card and its answer go by, the same each time it is read.
    pub fn id(&self) -> String {
        self.session.request_id(self.ask.key())
    }
}

fn words(list: &Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|word| crate::text(word).map(str::to_owned))
        .collect()
}
