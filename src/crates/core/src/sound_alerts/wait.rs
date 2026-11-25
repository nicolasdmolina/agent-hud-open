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
    /// The name it was decided under while it was unnamed, where that name
    /// is only the time of the session's last event and not a time of the
    /// wait's own (a channel's waiting flag has none). Once the wait is
    /// seen to end the name is marked as over, and the next wait goes by
    /// the name after it, so it is new though nothing was written between
    /// the two. The host plays an alert once by its name: the same name
    /// again would never be heard.
    pub lent: Option<String>,
}
