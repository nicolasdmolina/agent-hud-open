//! A decision made on a card, as the frame that tells Hermes. Each kind of
//! request builds only its own answer: a question takes words, an approval
//! one of the choices it offered, a set of questions an answer per question.
//! Nothing here can put an approval's choice on a question: a question has
//! no call that takes one.
//!
//! ```compile_fail
//! use agent_hud_agents::hermes::gateway::{Choice, Question, Scope, SessionRef};
//! fn approve(question: &Question, session: &SessionRef) {
//!     question.respond(session, Choice::Once, Scope::Profile);
//! }
//! ```
//!
//! The same call on an approval is the one that exists:
//!
//! ```
//! use agent_hud_agents::hermes::gateway::{Approval, Choice, Scope, SessionRef};
//! fn approve(approval: &Approval, session: &SessionRef) {
//!     let _ = approval.respond(session, Choice::Once, Scope::Profile);
//! }
//! ```
use super::ask::{Approval, Ask, Batch, Choice, Pending, Question};
use super::command::Command;
use super::identity::SessionRef;
use super::ticket::Scope;
use crate::l10n;
use serde_json::{json, Map, Value};

/// The words that answer one question, never empty: Hermes takes an empty
/// answer as no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer(String);

impl Answer {
    /// An answer in the person's own words.
    pub fn typed(words: &str) -> Option<Self> {
        let words = words.trim();
        (!words.is_empty()).then(|| Self(words.to_owned()))
    }
    /// The answer to a question from what was picked and typed on its card.
    /// One choice goes as its words, and so do words typed alone. Where
    /// several may be picked, the picks (and words typed with them) go as a
    /// JSON array in the string, which Hermes reads back as a list.
    pub fn given(selected: &[&str], custom: Option<&str>, multi_select: bool) -> Option<Self> {
        let custom = custom.map(str::trim).filter(|words| !words.is_empty());
        let picked: Vec<&str> = selected
            .iter()
            .map(|word| word.trim())
            .filter(|word| !word.is_empty())
            .collect();
        if multi_select && !picked.is_empty() {
            let all: Vec<&str> = picked.into_iter().chain(custom).collect();
            return Some(Self(json!(all).to_string()));
        }
        Self::typed(custom.or(picked.first().copied())?)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A choice Hermes did not offer for this approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotOffered(pub Choice);

/// What goes back to the gateway.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// A call, which the gateway answers: resolved, already resolved, refused.
    Call(Command),
    /// The whole result frame for a request of the gateway's own. Nothing
    /// answers it: the request is over once `request.cancel` says so, or a
    /// later reading no longer lists it.
    Result(Value),
}

impl Question {
    /// Skipping the question: Hermes's contract reads an empty answer as
    /// skipped, and the turn goes on with the question unanswered
    /// (`question-skipped`).
    pub fn skip(&self, session: &SessionRef, scope: Scope) -> Command {
        self.call(session, "", scope)
    }
    pub fn answer(&self, session: &SessionRef, answer: &Answer, scope: Scope) -> Command {
        self.call(session, answer.as_str(), scope)
    }
    fn call(&self, session: &SessionRef, answer: &str, scope: Scope) -> Command {
        Command::routed(
            "clarify.respond",
            json!({
                "session_id": session.id,
                "prompt_id": self.prompt.id,
                "execution_generation": self.prompt.generation,
                "answer": answer,
            }),
            &session.profile,
            scope,
        )
    }
}

impl Approval {
    pub fn respond(
        &self,
        session: &SessionRef,
        choice: Choice,
        scope: Scope,
    ) -> Result<Command, NotOffered> {
        if !self.choices.contains(&choice) {
            return Err(NotOffered(choice));
        }
        Ok(Command::routed(
            "approval.respond",
            json!({
                "session_id": session.id,
                "prompt_id": self.prompt.id,
                "execution_generation": self.prompt.generation,
                "choice": choice.word(),
            }),
            &session.profile,
            scope,
        ))
    }
}

impl Batch {
    /// The result frame that answers the questions given. A question left
    /// out keeps what another viewer answered, or stays unanswered. None
    /// when nothing is answered: a result without answers would cancel the
    /// whole set for every viewer.
    pub fn answer(&self, answers: &[(&str, Answer)]) -> Option<Value> {
        let written: Map<String, Value> = self
            .questions
            .iter()
            .filter_map(|question| {
                let (_, answer) = answers.iter().find(|(qid, _)| *qid == question.qid)?;
                Some((question.qid.clone(), json!(answer.as_str())))
            })
            .collect();
        (!written.is_empty())
            .then(|| json!({"jsonrpc": "2.0", "id": self.request, "result": {"answers": written}}))
    }
}

impl Pending {
    /// The reply for the decision made on its card (`RequestDecision`), on
    /// a socket of `scope`. None leaves the request for Hermes's own
    /// windows; an error leaves the card waiting.
    pub fn reply(&self, decision: &Value, scope: Scope) -> Result<Option<Reply>, String> {
        let kind = decision["kind"].as_str().unwrap_or("");
        if kind == "leave" {
            return Ok(None);
        }
        let empty = || l10n::text("请选择或输入回答", "Choose an option or type an answer.");
        // The card's Skip on every question it shows: no answer at all.
        let skipped = decision["answers"].as_object().is_some_and(Map::is_empty);
        match &self.ask {
            Ask::Question(_) | Ask::Batch(_) if kind != "answer" => Err(l10n::text(
                "Hermes 的提问只能回答，不能批准",
                "Hermes's questions can only be answered, not approved.",
            )),
            Ask::Question(question) if skipped => {
                Ok(Some(Reply::Call(question.skip(&self.session, scope))))
            }
            // A result without answers would cancel the whole set for every
            // viewer, so skipping all of it leaves it to them.
            Ask::Batch(_) if skipped => Ok(None),
            Ask::Question(question) => {
                let answer = written(decision, &question.prompt.id, question.multi_select)
                    .ok_or_else(empty)?;
                let call = question.answer(&self.session, &answer, scope);
                Ok(Some(Reply::Call(call)))
            }
            Ask::Batch(batch) => {
                let answers: Vec<(&str, Answer)> = batch
                    .questions
                    .iter()
                    .filter_map(|question| {
                        let answer = written(decision, &question.qid, question.multi_select)?;
                        Some((question.qid.as_str(), answer))
                    })
                    .collect();
                Ok(Some(Reply::Result(
                    batch.answer(&answers).ok_or_else(empty)?,
                )))
            }
            Ask::Approval(approval) => {
                let choice = match (kind, decision["suggestion"]["scope"].as_str()) {
                    ("allow", _) => Choice::Once,
                    ("deny", _) => Choice::Deny,
                    ("always", None) => Choice::Always,
                    ("always", Some("session")) => Choice::Session,
                    ("always", Some("always")) => Choice::Always,
                    _ => {
                        return Err(l10n::text(
                            "Hermes 的审批只能批准或拒绝",
                            "Hermes's approvals can only be approved or denied.",
                        ))
                    }
                };
                let call = approval
                    .respond(&self.session, choice, scope)
                    .map_err(|_| {
                        l10n::text(
                            "Hermes 没有为这个请求提供该选项",
                            "Hermes did not offer that choice for this request.",
                        )
                    })?;
                Ok(Some(Reply::Call(call)))
            }
        }
    }
}

/// The answer written on a card for the question with this answer key.
fn written(decision: &Value, key: &str, multi_select: bool) -> Option<Answer> {
    let given = &decision["answers"][key];
    let selected: Vec<&str> = given["selected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Answer::given(&selected, given["custom"].as_str(), multi_select)
}
