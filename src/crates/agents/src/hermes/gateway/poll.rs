//! One poll of a gateway: every question the connection must ask it to
//! know what waits now, and what it answered. The connection asks for
//! `steps()`, sends each as its `command`, gives each result to `answer`
//! (or says it `failed`), and asks again until there are none. Then the
//! whole poll goes to the model as `Input::Polled`.
//!
//! A poll lists the sessions of every served profile, then reads each
//! session that works, waits or shows a card (`read.rs`).
use super::command::{active_list, events_since, resume, Command};
use super::effect::Status;
use super::fold::{batches, prompts, Reading};
use super::frame::Cursor;
use super::identity::{GatewayId, Profile, SessionRef};
use super::read::Search;
use super::ticket::Scope;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// One question to the gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The live sessions of one profile (`session.active_list`).
    List(Profile),
    /// A session's events after `cursor`, or with none, where its stream is
    /// (`session.events.since`). Needs no subscription.
    Since {
        session: SessionRef,
        cursor: Option<Cursor>,
    },
    /// Everything that waits in a session (`session.resume`), on a socket
    /// of its own, closed once the reply is in.
    Snapshot(SessionRef),
}

impl Step {
    /// The call that asks it, on a socket of `scope`.
    pub fn command(&self, scope: Scope) -> Command {
        match self {
            Self::List(profile) => active_list(profile, scope),
            Self::Since { session, cursor } => events_since(session, cursor.as_ref(), scope),
            Self::Snapshot(session) => resume(session, scope),
        }
    }
}

/// The cards of one session when a poll began, by the gateway's ids.
#[derive(Debug, Default)]
pub(super) struct Shown {
    /// Questions and approvals.
    pub(super) prompts: Vec<String>,
    /// Sets of questions.
    pub(super) sets: Vec<String>,
}

#[derive(Debug)]
pub(super) struct Look {
    pub(super) status: Status,
    pub(super) read: Read,
    /// A snapshot of it was taken in this poll.
    pub(super) snapshot: bool,
}

#[derive(Debug)]
pub(super) enum Read {
    /// Neither working nor waiting and showing no card: not read.
    Skipped,
    /// `step` is still to be answered.
    Wanted {
        step: Step,
        search: Search,
    },
    Done(Reading),
    /// It could not be read this time: what it waits on is not known.
    Failed,
}

#[derive(Debug)]
pub struct Poll {
    pub(super) gateway: GatewayId,
    /// The model's count when the poll began: a poll that is not the
    /// newest when it comes back is dropped.
    pub(super) number: u64,
    /// Whether each served profile was listed; None until it answers.
    pub(super) profiles: BTreeMap<Profile, Option<bool>>,
    pub(super) sessions: BTreeMap<SessionRef, Look>,
    pub(super) shown: BTreeMap<SessionRef, Shown>,
    /// The sessions a snapshot was taken of too recently to take another.
    pub(super) recent: BTreeSet<SessionRef>,
}

impl Poll {
    pub(super) fn new(
        gateway: GatewayId,
        number: u64,
        served: impl Iterator<Item = Profile>,
        shown: BTreeMap<SessionRef, Shown>,
        recent: BTreeSet<SessionRef>,
    ) -> Self {
        Self {
            gateway,
            number,
            profiles: served.map(|profile| (profile, None)).collect(),
            sessions: BTreeMap::new(),
            shown,
            recent,
        }
    }

    /// What to ask now. Their order does not matter and they may be asked
    /// at once. Empty when the poll has run to its end.
    pub fn steps(&self) -> Vec<Step> {
        let lists = self
            .profiles
            .iter()
            .filter(|(_, listed)| listed.is_none())
            .map(|(profile, _)| Step::List(profile.clone()));
        let reads = self.sessions.values().filter_map(|look| match &look.read {
            Read::Wanted { step, .. } => Some(step.clone()),
            _ => None,
        });
        lists.chain(reads).collect()
    }

    /// The `result` of the reply to `step`.
    pub fn answer(&mut self, step: &Step, result: &Value) {
        match step {
            Step::List(profile) => self.listed(profile, result),
            Step::Since { session, cursor } => {
                if let Some(search) = self.wanted(session, step) {
                    self.read(session, cursor.as_ref(), search, result);
                }
            }
            Step::Snapshot(session) => {
                let Some(search) = self.wanted(session, step) else {
                    return;
                };
                let named = result["session_id"]
                    .as_str()
                    .is_none_or(|id| id == session.id());
                let Some(mut asks) = prompts(&result["prompts"]).filter(|_| named) else {
                    return self.set(session, Read::Failed);
                };
                asks.extend(batches(&result["open_requests"]));
                let reading = Reading {
                    asks,
                    over: search.over,
                    ..Reading::default()
                };
                if let Some(look) = self.sessions.get_mut(session) {
                    look.snapshot = true;
                    look.read = Read::Done(reading);
                }
            }
        }
    }

    /// `step` was refused or never answered. What it would have told stays
    /// unknown for this poll: the cards it concerns are left as they are.
    pub fn failed(&mut self, step: &Step) {
        match step {
            Step::List(profile) => {
                if let Some(listed @ None) = self.profiles.get_mut(profile) {
                    *listed = Some(false);
                }
            }
            Step::Since { session, .. } | Step::Snapshot(session) => {
                if self.wanted(session, step).is_some() {
                    self.set(session, Read::Failed);
                }
            }
        }
    }

    fn listed(&mut self, profile: &Profile, result: &Value) {
        let Some(listed @ None) = self.profiles.get_mut(profile) else {
            return;
        };
        let Some(rows) = result["sessions"].as_array() else {
            *listed = Some(false);
            return;
        };
        *listed = Some(true);
        for row in rows {
            let Some(id) = crate::text(&row["id"]).or_else(|| crate::text(&row["session_id"]))
            else {
                continue;
            };
            // A row whose id can be no session's is not followed.
            let Some(session) = SessionRef::new(self.gateway.clone(), profile.clone(), id) else {
                continue;
            };
            // A word Hermes adds later is taken as working: the session is read.
            let status = row["status"]
                .as_str()
                .and_then(Status::parse)
                .unwrap_or(Status::Working);
            let busy = matches!(status, Status::Working | Status::Waiting);
            let read = if busy || self.shown.contains_key(&session) {
                let (session, cursor) = (session.clone(), None);
                let step = Step::Since { session, cursor };
                let search = Search::default();
                Read::Wanted { step, search }
            } else {
                Read::Skipped
            };
            let snapshot = false;
            let look = Look {
                status,
                read,
                snapshot,
            };
            self.sessions.insert(session, look);
        }
    }

    /// The session's search so far, if `step` is the one it waits on.
    pub(super) fn wanted(&mut self, at: &SessionRef, step: &Step) -> Option<Search> {
        match &mut self.sessions.get_mut(at)?.read {
            Read::Wanted {
                step: asked,
                search,
            } if asked == step => Some(std::mem::take(search)),
            _ => None,
        }
    }

    pub(super) fn set(&mut self, at: &SessionRef, read: Read) {
        if let Some(look) = self.sessions.get_mut(at) {
            look.read = read;
        }
    }
}
