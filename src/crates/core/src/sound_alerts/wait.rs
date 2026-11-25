//! What the sound decision keeps of a session's wait from one look to the
//! next: what it was waiting on, and under which name it was decided. It
//! keeps no approval to decide by: what is approved is what core says at
//! each look (ticket 66, round 6).
use super::requests::Request;

/// What a session was waiting on at the last look: its requests (none for a
/// wait its hooks or process report without a card).
pub(super) struct Wait {
    pub requests: Vec<Request>,
    /// The requests its channel said the wait was for, by their ids.
    pub named: Vec<String>,
    /// Those of them core said were allowed automatically, their answer
    /// confirmed. Kept only to tell core when their wait is seen to end
    /// (`ended`): no decision reads it.
    pub approved: Vec<String>,
    /// It had no card and was for approved requests alone, so it passed
    /// without a sound: a card that comes in it is new, and so is a wait
    /// that names no request. It only ever lets what follows sound.
    pub silent: bool,
    /// The name it was decided under while it had no card, where that name
    /// is only the time of the session's last event and not a time of the
    /// wait's own (a channel's waiting flag has none). Once the wait is
    /// seen to end the name is marked as over, and the next wait goes by
    /// the name after it, so it is new though nothing was written between
    /// the two. The host plays an alert once by its name: the same name
    /// again would never be heard.
    pub lent: Option<String>,
}
impl Wait {
    /// Whether `request`, a card that comes while this wait had none, can
    /// be what the wait was for: not where the channel named the wait's
    /// requests and this card is none of them.
    pub fn may_be(&self, request: &Request) -> bool {
        let named = |name| self.named.contains(name);
        !request.card || self.named.is_empty() || request.names.iter().any(named)
    }
}
