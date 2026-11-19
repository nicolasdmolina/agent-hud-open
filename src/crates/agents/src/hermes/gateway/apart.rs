//! One session read apart from the polls. A session whose events take
//! long to read holds up every poll that reads it, and with the poll,
//! every other session's cards. So the connection may read such a session
//! by itself, on a socket of its own, while the polls go on without it:
//! a poll is told that session's read failed, which leaves its cards as
//! they are, and the reading apart is what opens, changes and closes
//! them.
//!
//! A reading apart is a poll of one session and follows a poll's rules.
//! It reads the session from the oldest event kept, takes a snapshot
//! where the events cannot say, and is whole or not used. It is dropped
//! where it was begun before something it cannot know of: an answer
//! Hermes took, a restart, a dropped socket, a profile that left the
//! gateway, a newer reading of the same session, the session leaving
//! the live list, or a poll listing the session as something else that
//! is not waiting. It never lists anything: what a session is doing is
//! the polls' to say, and a reading apart goes by what the newest poll
//! listed it as when it began. So a reading begun while the session
//! waited opens nothing once a poll has listed it idle, or working: what
//! it saw waiting was settled since, as far as it can know.
//!
//! A poll that lists the session as waiting where it worked drops no
//! reading: a request was asked since, and a reading under way either
//! read it, and its card shows that much sooner, or did not, and opens
//! nothing. Dropping it would make every card of a session that is slow
//! to read wait for a reading begun after the poll.
//!
//! Whether a reading still counts can be asked while it is under way
//! (`Gateway::counts`), so that whoever runs one sends nothing more for
//! a reading that is of no use: no further read, and above all no
//! snapshot, which subscribes to a session.
use super::ask::Ask;
use super::effect::{Effect, Status};
use super::identity::SessionRef;
use super::poll::{Look, Poll, Read, Shown, Step};
use super::read::Search;
use super::state::{Gateway, SNAPSHOT_EVERY_MS};
use super::Connection;
use std::collections::{BTreeMap, BTreeSet};

/// What makes a poll a reading of one session apart.
#[derive(Clone, Debug)]
pub(super) struct Apart {
    pub(super) session: SessionRef,
    /// The model's count of changes when it began.
    changes: u64,
    /// Which reading apart it is, counted over all sessions.
    number: u64,
    /// The listing of the session it began under: the poll that last
    /// listed it anew, or as something else that is not waiting.
    listing: Option<u64>,
}

impl Gateway {
    /// Begins a reading of `session` alone, which is the newest of that
    /// session from now on. None while the gateway is not ready, where no
    /// poll lists the session as live, and where a poll would not read it
    /// either: it neither works nor waits and shows no card.
    pub fn poll_apart(&mut self, session: &SessionRef) -> Option<Poll> {
        if !matches!(self.connection, Connection::Ready(_)) || session.gateway != self.id {
            return None;
        }
        let status = *self.listed.get(session)?;
        let mut shown = Shown::default();
        for ((_, key), card) in self.cards.iter().filter(|((at, _), _)| at == session) {
            match card.pending.ask {
                Ask::Batch(_) => shown.sets.push(key.clone()),
                Ask::Question(_) | Ask::Approval(_) => shown.prompts.push(key.clone()),
            }
        }
        let cards = !shown.prompts.is_empty() || !shown.sets.is_empty();
        if !matches!(status, Status::Working | Status::Waiting) && !cards {
            return None;
        }
        self.readings += 1;
        self.apart.insert(session.clone(), self.readings);
        let apart = Apart {
            session: session.clone(),
            changes: self.changes,
            number: self.readings,
            listing: self.listing.get(session).copied(),
        };
        let step = Step::Since {
            session: session.clone(),
            cursor: None,
        };
        let look = Look {
            status,
            listed: Default::default(),
            read: Read::Wanted {
                step,
                search: Search::default(),
            },
            snapshot: false,
        };
        let taken = self.snapshotted.get(session);
        let recent = taken.is_some_and(|at| self.now_ms - *at < SNAPSHOT_EVERY_MS);
        Some(Poll {
            gateway: self.id.clone(),
            number: self.polls,
            profiles: BTreeMap::from([(session.profile.clone(), Some(true))]),
            sessions: BTreeMap::from([(session.clone(), look)]),
            shown: BTreeMap::from([(session.clone(), shown)]),
            recent: recent.then(|| session.clone()).into_iter().collect(),
            apart: Some(Box::new(apart)),
            turns: self.turns.watch(),
            places: BTreeMap::new(),
        })
    }

    /// Whether `poll` still counts: whether the model would take what it
    /// read. For a poll, that it is the newest and the gateway is ready.
    /// For a reading of one session apart, also that nothing happened
    /// since it began that it cannot know of, that it is the newest
    /// reading of its session, that the session is still on the live
    /// list, and that no poll has since given it a new listing.
    pub fn counts(&self, poll: &Poll) -> bool {
        let ready = matches!(self.connection, Connection::Ready(_));
        if poll.gateway != self.id || !ready {
            return false;
        }
        let Some(apart) = &poll.apart else {
            return poll.number == self.polls;
        };
        let at = &apart.session;
        let newest = self.apart.get(at) == Some(&apart.number);
        let listed = self.listed.contains_key(at) && self.listing.get(at) == apart.listing.as_ref();
        newest && listed && apart.changes == self.changes
    }

    /// Makes the cards of one session match what a reading of it alone
    /// found, where that reading still counts.
    pub(super) fn reconcile_apart(
        &mut self,
        poll: Poll,
        apart: &Apart,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        if !self.counts(&poll) {
            return;
        }
        self.watched(&poll);
        let at = &apart.session;
        self.stilled_apart(&poll, at, now_ms);
        let its: Vec<_> = self
            .cards
            .keys()
            .filter(|(of, _)| of == at)
            .cloned()
            .collect();
        for key in its {
            if let Some(why) = self.over(&poll, &key, now_ms, effects) {
                self.close(&key, why, effects);
            }
        }
        // After the cards, as in a poll: a card that closes forgets its
        // session's snapshot, and one taken in this reading is newer.
        let look = poll.sessions.get(at);
        if look.is_some_and(|look| look.snapshot && look.status == Status::Waiting) {
            self.snapshotted.insert(at.clone(), now_ms);
        }
        self.open(poll.sessions, effects);
    }

    /// Something happened that no reading begun before it can know of.
    pub(super) fn changed(&mut self) {
        self.changes += 1;
    }

    /// A poll listed `at` anew, or as no longer doing what the poll
    /// before listed and not as waiting: its listing is that poll's from
    /// now on, and a reading of it begun under the one before counts no
    /// more.
    pub(super) fn relisted(&mut self, at: &SessionRef, poll: u64) {
        self.listing.insert(at.clone(), poll);
    }

    /// The sessions the newest poll no longer lists are read apart no
    /// more, and have no listing: a reading of one that is still out
    /// counts for nothing, also once the session is listed again.
    pub(super) fn keep_apart(&mut self) {
        let listed: BTreeSet<&SessionRef> = self.listed.keys().collect();
        self.apart.retain(|at, _| listed.contains(at));
        self.listing.retain(|at, _| listed.contains(at));
    }
}
