//! Makes the cards match what one poll found. For every session the poll
//! read whole, the requests that wait are exactly the ones it found: a
//! card it did not find closes, one it found changed is shown as it is
//! now, and a request with no card opens one. A session that left the live
//! list is over with its cards. Where the poll could not tell (a list or a
//! read that failed), the cards are left as they are and the next poll
//! asks again.
use super::ask::{Ask, Pending};
use super::effect::{Closed, Effect, Status};
use super::identity::SessionRef;
use super::poll::{Poll, Read};
use super::state::{Card, Gateway, Key};
use super::Connection;

/// How long a sent answer to a set of questions may go unconfirmed before
/// a poll that still lists the set means the answer was dropped, which
/// Hermes does without a word. It closes nothing: the card is shown again.
const UNCONFIRMED_MS: i64 = 5_000;

impl Gateway {
    pub(super) fn reconcile(&mut self, poll: Poll, now_ms: i64, effects: &mut Vec<Effect>) {
        // Only this gateway's newest poll counts, and only while it is
        // ready. Any other was begun before something it cannot know of: a
        // newer poll, an answer Hermes took, a restart, a dropped socket.
        let ready = matches!(self.connection, Connection::Ready(_));
        if poll.gateway != self.id || poll.number != self.polls || !ready {
            return;
        }
        // What each profile that answered lists replaces what was known of
        // it; a profile no longer served has no live sessions; one whose
        // list failed keeps what was known.
        let unlisted = |listed: Option<&Option<bool>>| matches!(listed, Some(None | Some(false)));
        self.listed
            .retain(|at, _| unlisted(poll.profiles.get(&at.profile)));
        self.told.read(&poll, now_ms);
        for (at, look) in &poll.sessions {
            self.listed.insert(at.clone(), look.status);
        }
        for key in self.cards.keys().cloned().collect::<Vec<_>>() {
            if let Some(why) = self.over(&poll, &key, now_ms, effects) {
                self.close(&key, why, effects);
            }
        }
        // After the cards, since a card that closes forgets its session's
        // snapshot and a snapshot taken in this poll is newer than that.
        // A session that no longer waits, or left the list, has none.
        let waits = |at: &SessionRef| {
            let look = poll.sessions.get(at);
            look.is_some_and(|look| look.status == Status::Waiting)
        };
        self.snapshotted
            .retain(|at, _| unlisted(poll.profiles.get(&at.profile)) || waits(at));
        for (at, look) in &poll.sessions {
            if look.snapshot && look.status == Status::Waiting {
                self.snapshotted.insert(at.clone(), now_ms);
            }
        }
        for (at, look) in poll.sessions {
            let Read::Done(reading) = look.read else {
                continue;
            };
            // No card opens for a profile two instances claim (`served.rs`).
            if self.withheld.contains(&at.profile) {
                continue;
            }
            for (ask, wait_ms) in reading.asks {
                let key = (at.clone(), ask.key().to_owned());
                if self.cards.contains_key(&key) {
                    continue;
                }
                let pending = Pending {
                    session: at.clone(),
                    ask,
                    wait_ms,
                };
                effects.push(Effect::Open(pending.clone()));
                let (sent_ms, marked) = (None, false);
                let card = Card {
                    pending,
                    sent_ms,
                    marked,
                };
                self.cards.insert(key, card);
            }
        }
    }

    /// Why the card is over by this poll, or None where it still waits or
    /// the poll cannot tell. A card that still waits is brought up to date.
    fn over(
        &mut self,
        poll: &Poll,
        key: &Key,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) -> Option<Closed> {
        let (at, asked) = key;
        match poll.profiles.get(&at.profile) {
            // The gateway no longer serves the profile.
            None => return Some(Closed::SessionEnded),
            Some(Some(true)) => {}
            Some(_) => return None,
        }
        let Some(look) = poll.sessions.get(at) else {
            return Some(Closed::SessionEnded);
        };
        let Read::Done(reading) = &look.read else {
            return None;
        };
        let Some((ask, wait_ms)) = reading.asks.iter().find(|(ask, _)| ask.key() == asked) else {
            if reading.kept.contains(asked) {
                return None;
            }
            return Some(reading.over.get(asked).cloned().unwrap_or(Closed::Gone));
        };
        let card = self.cards.get_mut(key)?;
        if card.pending.ask != *ask || card.pending.wait_ms != *wait_ms {
            card.pending.ask = ask.clone();
            card.pending.wait_ms = *wait_ms;
            effects.push(Effect::Changed(card.pending.clone()));
        }
        // Read whole, and the request waits: it can be answered again.
        card.in_reach(effects);
        let dropped = matches!(card.pending.ask, Ask::Batch(_))
            && card
                .sent_ms
                .is_some_and(|sent| now_ms - sent >= UNCONFIRMED_MS);
        if dropped {
            card.sent_ms = None;
            effects.push(Effect::Unanswered {
                pending: card.pending.clone(),
                why: None,
            });
        }
        None
    }
}
