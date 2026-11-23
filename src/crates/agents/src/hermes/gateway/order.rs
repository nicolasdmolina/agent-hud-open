//! The order one poll asks in (tickets 72 and 73). A poll goes to the
//! model whole, once every call of it is answered or given up, and a
//! gateway answers the calls of one socket in turn. So one call that is
//! slow holds up the card of every session the poll read, and every call
//! sent after it.
//!
//! Two kinds of call are for sessions nothing has changed in: the read
//! of a session that has stood still for the quiet time (`quiet.rs`),
//! and the look at an idle session's stream, on which no card rests
//! (`turns.rs`). A poll asks those last, once the lists and every other
//! read are answered to their end: a session that just asked is read
//! whole before any of them goes out, and never queues behind one. It
//! says when it has reached them (`last`), and the connection gives them
//! a short time together where it gives any other call a long one. A
//! far gateway where nothing moves is polled every ten seconds, and a
//! card there is to show within thirteen.
//!
//! Of those, the reads go first, to their end, and only then the looks.
//! A card can wait on the read of a quiet session after all: Hermes
//! goes on listing a session as working while a set of questions waits
//! in it, and the set may open with no event, so nothing but the answer
//! to that read tells of it. Asked in one round with the looks, a look
//! that was never answered held the round until the short time was up,
//! and the session's next read was never asked.
//!
//! A quiet session is still read at every poll, as before. Its read is
//! among the first again the moment the list says something else of it,
//! and for as long as a card of it shows. On this machine nothing is
//! asked last: its polls ask as they did.
use super::identity::SessionRef;
use super::poll::{Look, Poll, Read, Step};

impl Poll {
    /// Whether the read of `at` is asked last: nothing had changed in
    /// the session for the quiet time when the poll began, the list says
    /// of it what it said then, and no card of it shows.
    fn stands(&self, at: &SessionRef, look: &Look) -> bool {
        self.still.get(at) == Some(&look.status) && !self.shown.contains_key(at)
    }

    /// The reads still to be answered, of the sessions that stand still
    /// or of the others.
    fn reads(&self, still: bool) -> impl Iterator<Item = Step> + '_ {
        let wanted = move |(at, look): (&SessionRef, &Look)| match &look.read {
            Read::Wanted { step, .. } if self.stands(at, look) == still => Some(step.clone()),
            _ => None,
        };
        self.sessions.iter().filter_map(wanted)
    }

    /// The lists, and the reads a card that is new may wait on.
    fn first(&self) -> Vec<Step> {
        let unlisted = |(_, listed): &(&_, &Option<bool>)| listed.is_none();
        let lists = self.profiles.iter().filter(unlisted);
        let lists = lists.map(|(profile, _)| Step::List(profile.clone()));
        lists.chain(self.reads(false)).collect()
    }

    /// What to ask now. The steps of one answer do not depend on each
    /// other and may be asked at once, in this order. Empty when the
    /// poll has run to its end.
    pub fn steps(&self) -> Vec<Step> {
        let first = self.first();
        if !first.is_empty() {
            return first;
        }
        let still: Vec<Step> = self.reads(true).collect();
        if !still.is_empty() {
            return still;
        }
        let looks = self.turns.iter().flat_map(|watch| watch.steps());
        looks.collect()
    }

    /// Whether what `steps` asks now is asked last: nothing but calls
    /// for sessions in which nothing has changed, which a poll is not to
    /// be held up by for long.
    pub fn last(&self) -> bool {
        self.first().is_empty()
    }

    /// Gives up everything that is still to be asked: what it would
    /// have told stays unknown for this poll, which is then at its end.
    /// Returns what was given up, in the order it would have been asked.
    pub fn give_up(&mut self) -> Vec<Step> {
        let mut left = Vec::new();
        // The first calls, then the quiet sessions' reads, then the
        // looks: each comes up once what is before it is given up.
        for _ in 0..3 {
            for step in self.steps() {
                self.failed(&step);
                left.push(step);
            }
        }
        left
    }
}
