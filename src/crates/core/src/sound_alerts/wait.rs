//! What the sound decision keeps of a session's wait from one look to the
//! next: what it was waiting on, and under which name it was decided. It
//! keeps no approval to decide by: what is approved is what core says at
//! each look (ticket 66, round 6).

/// What a session was waiting on at the last look.
pub(super) struct Wait {
    /// Nothing of its own waited in it: no card, no transcript sighting,
    /// and no request its channel named that was not approved. It was a
    /// wait known only as a wait, told (if at all) under a name of the
    /// session's. A card that comes in it is taken for that same wait.
    pub unnamed: bool,
    /// The requests its channel said the wait was for, each with the name
    /// it went by: kept to mark that name as over once the wait names the
    /// request no more (`Tracker::request_names`).
    pub named: Vec<(String, String)>,
    /// Every ask its channel said waited in it, each with the name it went
    /// by: kept to mark that name as over once its request is gone
    /// (`asks.rs`).
    pub asks: Vec<super::asks::Ask>,
    /// The requests its channel said the wait was for that core said were
    /// allowed automatically, their answer confirmed. Kept only to tell
    /// core when their wait is seen to end (`ended`): no decision reads it.
    pub approved: Vec<String>,
    /// It had no card and was for approved requests alone, so it passed
    /// without a sound: a card that comes in it is new, and so is a wait
    /// that names no request. It only ever lets what follows sound.
    pub silent: bool,
    /// The base of the name it was decided under while it was unnamed,
    /// where that is only the time of the session's last event and not a
    /// time of the wait's own (a channel's waiting flag has none). Once the
    /// wait is seen to end, the base's count goes up, and the next wait
    /// goes by a name no wait had, though nothing was written between the
    /// two (`bare.rs`).
    pub lent: Option<String>,
    /// The names of its requests and asks that waited with no card and
    /// were not approved: once one of them ends under its plain name, the
    /// name is forgotten, so a card of that request afterwards is another
    /// wait and sounds (`Tracker::spent`).
    pub uncarded: Vec<String>,
    /// The names of its requests that showed a card. Once the wait is seen
    /// to end at a look they are over too, as its other names are: a wait
    /// of that request with no card afterwards goes by the name after and
    /// sounds, also where the client did not name it for the wait; a card
    /// shown again within the day is still the same card (ticket 81).
    pub cards: Vec<String>,
}

impl Wait {
    /// Whether one of `names` (a card's, in session `key`) is the plain
    /// name of a request or ask that waited in this wait with no card: the
    /// card is then that same wait, whatever count its no-card name had.
    pub(super) fn uncarded_as(&self, key: &str, names: &[String]) -> bool {
        let quiet = |name: &String| !self.uncarded.contains(name);
        let requests = self.named.iter().filter(|(_, n)| !quiet(n)).map(|(r, _)| r);
        let asks = self.asks.iter().filter(|a| !quiet(&a.name)).map(|a| &a.ask);
        let mut bases = requests.chain(asks).map(|r| format!("needs-you:{key}:{r}"));
        bases.any(|base| names.contains(&base))
    }
}
