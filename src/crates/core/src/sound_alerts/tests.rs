use super::*;

const T: i64 = 1_800_000_000_000;

fn on() -> Value {
    json!({"sound":true,"soundNeedsYou":"Glass","soundFinished":"Hero"})
}
fn off() -> Value {
    json!({"sound":false,"soundNeedsYou":"Glass","soundFinished":"Hero"})
}
fn no_events() -> Value {
    json!({"revision":0,"items":[]})
}
/// A session row as the snapshot shows it, waiting on these requests.
fn waiting(id: &str, requests: &[(&str, &str)]) -> Value {
    let requests: Vec<Value> = requests
        .iter()
        .map(|(rid, kind)| json!({"id":rid,"sessionId":id,"kind":kind,"at":"2027-01-15T08:00:00Z"}))
        .collect();
    json!({"id":id,"source":"claude","state":"waiting","liveFacts":{"observedAt":"2027-01-15T08:00:00Z","requests":requests,"activity":null}})
}
/// A session at work on a step: a tool call open, nothing asked.
fn running(id: &str) -> Value {
    json!({"id":id,"source":"claude","state":"running","liveFacts":{"observedAt":"2027-01-15T08:00:00Z","requests":[],"activity":{"work":"tool","openCall":{"name":"Bash"},"toolAt":"2027-01-15T08:00:00Z"}}})
}
/// The island's history holding a turn of `session` that ended.
fn completed(session: &str, turn: &str) -> Value {
    json!({"revision":1,"items":[{"id":format!("event:1:{turn}"),"kind":"completion","at":"2027-01-15T08:00:00Z","source":"codex","sessionId":session,"turnId":turn}]})
}
/// A tracker past its first look, which only notes what already waits.
fn started() -> Tracker {
    let mut tracker = Tracker::new();
    tracker.update(&no_events(), &[], &on(), T);
    tracker
}
/// The alerts the look adds, as (kind, session, sound).
fn look(
    tracker: &mut Tracker,
    island: &Value,
    sessions: &[Value],
    settings: &Value,
    now: i64,
) -> Vec<(String, String, String)> {
    let before: HashSet<String> = tracker
        .items
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    let out = tracker.update(island, sessions, settings, now);
    out["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| !before.contains(item["id"].as_str().unwrap()))
        .map(|item| {
            let text = |key: &str| item[key].as_str().unwrap().to_owned();
            (text("kind"), text("sessionId"), text("sound"))
        })
        .collect()
}
fn needs_you(session: &str) -> (String, String, String) {
    ("needs-you".into(), session.into(), "Glass".into())
}

#[test]
fn a_request_appearing_needs_you() {
    let mut tracker = started();
    let sessions = [waiting("s1", &[("card-1", "permission")])];
    let alerts = look(&mut tracker, &no_events(), &sessions, &on(), T + 1);
    assert_eq!(alerts, [needs_you("s1")]);
    let item = tracker.items.back().unwrap();
    assert_eq!(item["source"], "claude");
    assert_eq!(item["id"], "needs-you:claude:s1:card-1");
}

#[test]
fn a_question_appearing_needs_you() {
    let mut tracker = started();
    let sessions = [waiting("s1", &[("log:claude:s1:call-9", "question")])];
    let alerts = look(&mut tracker, &no_events(), &sessions, &on(), T + 1);
    assert_eq!(alerts, [needs_you("s1")]);
}

#[test]
fn a_finished_turn_sounds_finished_once() {
    let mut tracker = started();
    let island = completed("s2", "turn-1");
    let alerts = look(&mut tracker, &island, &[], &on(), T + 1);
    assert_eq!(alerts, [("finished".into(), "s2".into(), "Hero".into())]);
    assert_eq!(tracker.items.back().unwrap()["source"], "codex");
    // The island keeps the completion in its history: no second sound.
    assert!(look(&mut tracker, &island, &[], &on(), T + 2).is_empty());
}

#[test]
fn a_step_or_tool_call_makes_no_sound() {
    let mut tracker = started();
    let sessions = [running("s1")];
    for at in 1..5 {
        assert!(look(&mut tracker, &no_events(), &sessions, &on(), T + at).is_empty());
    }
}

#[test]
fn the_same_request_across_refreshes_sounds_once() {
    let mut tracker = started();
    let sessions = [waiting("s1", &[("card-1", "permission")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &sessions, &on(), T + 1).len(),
        1
    );
    for at in 2..6 {
        assert!(look(&mut tracker, &no_events(), &sessions, &on(), T + at).is_empty());
    }
    // The same look given twice adds nothing either.
    let first = tracker.update(&no_events(), &sessions, &on(), T + 6);
    assert_eq!(tracker.update(&no_events(), &sessions, &on(), T + 6), first);
}

#[test]
fn an_answered_request_then_a_new_one_sounds_again() {
    let mut tracker = started();
    let a = [waiting("s1", &[("card-a", "permission")])];
    let b = [waiting("s1", &[("card-b", "permission")])];
    assert_eq!(look(&mut tracker, &no_events(), &a, &on(), T + 1).len(), 1);
    // Answered, the session works on, then asks again.
    assert!(look(&mut tracker, &no_events(), &[running("s1")], &on(), T + 2).is_empty());
    assert_eq!(
        look(&mut tracker, &no_events(), &b, &on(), T + 3),
        [needs_you("s1")]
    );
    // Answered and asked again between two looks.
    let c = [waiting("s1", &[("card-c", "question")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &c, &on(), T + 4),
        [needs_you("s1")]
    );
}

#[test]
fn a_second_request_waiting_beside_the_first_sounds_again() {
    let mut tracker = started();
    let one = [waiting("s1", &[("card-a", "permission")])];
    let two = [waiting(
        "s1",
        &[("card-a", "permission"), ("card-b", "permission")],
    )];
    assert_eq!(
        look(&mut tracker, &no_events(), &one, &on(), T + 1).len(),
        1
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &two, &on(), T + 2),
        [needs_you("s1")]
    );
}

#[test]
fn the_transcripts_sighting_and_its_card_are_one_wait() {
    let mut tracker = started();
    let seen = [waiting("s1", &[("log:claude:s1:call-1", "permission")])];
    let card = [waiting("s1", &[("card-1", "permission")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &seen, &on(), T + 1).len(),
        1
    );
    assert!(look(&mut tracker, &no_events(), &card, &on(), T + 2).is_empty());
    // The card runs out and the transcript shows the call waiting still.
    assert!(look(&mut tracker, &no_events(), &seen, &on(), T + 3).is_empty());
    let mut tracker = started();
    assert_eq!(
        look(&mut tracker, &no_events(), &card, &on(), T + 1).len(),
        1
    );
    let other = [waiting("s1", &[("log:claude:s1:call-2", "permission")])];
    assert!(look(&mut tracker, &no_events(), &other, &on(), T + 2).is_empty());
}

#[test]
fn a_wait_without_a_card_needs_you_once_and_its_card_adds_nothing() {
    let mut tracker = started();
    let mut asked = waiting("s1", &[]);
    asked["liveFacts"]["attention"] = json!({"at":"2027-01-15T08:00:05Z","message":"Allow?"});
    assert_eq!(
        look(&mut tracker, &no_events(), &[asked.clone()], &on(), T + 1),
        [needs_you("s1")]
    );
    assert!(look(&mut tracker, &no_events(), &[asked], &on(), T + 2).is_empty());
    let card = [waiting("s1", &[("card-1", "permission")])];
    assert!(look(&mut tracker, &no_events(), &card, &on(), T + 3).is_empty());
}

#[test]
fn with_sound_off_nothing_is_produced() {
    let mut tracker = started();
    let sessions = [waiting("s1", &[("card-1", "permission")])];
    let island = completed("s2", "turn-1");
    let out = tracker.update(&island, &sessions, &off(), T + 1);
    assert_eq!(out["items"], json!([]));
    // Turned on while that request still waits and the completion is still
    // in the island's history: neither sounds late.
    assert!(look(&mut tracker, &island, &sessions, &on(), T + 2).is_empty());
    // A setting with no switch at all is off.
    let out = tracker.update(&completed("s3", "turn-2"), &[], &json!({}), T + 3);
    assert_eq!(out["items"], json!([]));
}

#[test]
fn what_waits_at_the_first_look_is_not_told() {
    let mut tracker = Tracker::new();
    let sessions = [waiting("s1", &[("card-1", "permission")])];
    assert!(look(&mut tracker, &no_events(), &sessions, &on(), T).is_empty());
    assert!(look(&mut tracker, &no_events(), &sessions, &on(), T + 1).is_empty());
}

#[test]
fn the_chosen_sound_names_ride_on_each_alert() {
    let mut tracker = started();
    let settings = json!({"sound":true,"soundNeedsYou":"Ping","soundFinished":"Submarine"});
    let sessions = [waiting("s1", &[("card-1", "permission")])];
    let alerts = look(
        &mut tracker,
        &completed("s2", "turn-1"),
        &sessions,
        &settings,
        T + 1,
    );
    assert_eq!(
        alerts,
        [
            ("finished".into(), "s2".into(), "Submarine".into()),
            ("needs-you".into(), "s1".into(), "Ping".into())
        ]
    );
}

/// A session waiting on one request of `kind` opened at `at`.
fn waiting_since(id: &str, request: &str, kind: &str, at: &str) -> Value {
    let mut session = waiting(id, &[(request, kind)]);
    session["liveFacts"]["requests"][0]["at"] = json!(at);
    session
}

#[test]
fn distinct_transcript_questions_between_refreshes_both_sound() {
    let mut tracker = started();
    let a = [waiting("s1", &[("log:claude:s1:call-a", "question")])];
    let b = [waiting("s1", &[("log:claude:s1:call-b", "question")])];
    assert_eq!(look(&mut tracker, &no_events(), &a, &on(), T + 1).len(), 1);
    // A answered and a different question B asked between two looks.
    assert_eq!(
        look(&mut tracker, &no_events(), &b, &on(), T + 2),
        [needs_you("s1")]
    );
}

#[test]
fn a_card_opened_well_after_an_answered_sighting_sounds() {
    let mut tracker = started();
    let seen = [waiting_since(
        "s1",
        "log:claude:s1:call-a",
        "permission",
        "2027-01-15T08:00:00Z",
    )];
    let card = [waiting_since(
        "s1",
        "card-b",
        "permission",
        "2027-01-15T08:01:00Z",
    )];
    assert_eq!(
        look(&mut tracker, &no_events(), &seen, &on(), T + 1).len(),
        1
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &card, &on(), T + 2),
        [needs_you("s1")]
    );
    // The same, card first: a later sighting of another call sounds.
    let mut tracker = started();
    let card = [waiting_since(
        "s1",
        "card-a",
        "permission",
        "2027-01-15T08:00:00Z",
    )];
    let seen = [waiting_since(
        "s1",
        "log:claude:s1:call-b",
        "permission",
        "2027-01-15T08:01:00Z",
    )];
    assert_eq!(
        look(&mut tracker, &no_events(), &card, &on(), T + 1).len(),
        1
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &seen, &on(), T + 2),
        [needs_you("s1")]
    );
}

#[test]
fn every_alert_of_one_look_reaches_the_snapshot() {
    let mut tracker = started();
    let sessions: Vec<_> = (0..17)
        .map(|i| waiting(&format!("s{i}"), &[("card", "permission")]))
        .collect();
    let out = tracker.update(&no_events(), &sessions, &on(), T + 1);
    assert_eq!(out["revision"], 17);
    assert_eq!(out["items"].as_array().unwrap().len(), 17);
    // Forty in one look are all kept too; the next look keeps the island's
    // count of earlier ones beside its own.
    let sessions: Vec<_> = (0..40)
        .map(|i| waiting(&format!("t{i}"), &[("card", "permission")]))
        .collect();
    let out = tracker.update(&no_events(), &sessions, &on(), T + 2);
    assert_eq!(out["items"].as_array().unwrap().len(), 57);
    let out = tracker.update(
        &no_events(),
        &[waiting("u", &[("card", "question")])],
        &on(),
        T + 3,
    );
    let items = out["items"].as_array().unwrap();
    assert_eq!(items.len(), 33);
    assert_eq!(items.last().unwrap()["sessionId"], "u");
}

#[test]
fn two_new_requests_in_one_session_at_one_look_make_one_sound() {
    let mut tracker = started();
    let both = [waiting(
        "s1",
        &[("card-a", "permission"), ("card-b", "question")],
    )];
    assert_eq!(
        look(&mut tracker, &no_events(), &both, &on(), T + 1),
        [needs_you("s1")]
    );
    // Neither sounds again, alone or together.
    assert!(look(&mut tracker, &no_events(), &both, &on(), T + 2).is_empty());
    let b = [waiting("s1", &[("card-b", "question")])];
    assert!(look(&mut tracker, &no_events(), &b, &on(), T + 3).is_empty());
}

#[test]
fn a_continuous_wait_whose_observed_time_moves_sounds_once() {
    let mut tracker = started();
    for i in 1..=10 {
        let mut session = waiting("s1", &[]);
        session["liveFacts"]["observedAt"] = iso(T + i);
        let alerts = look(&mut tracker, &no_events(), &[session], &on(), T + i);
        assert_eq!(alerts.len(), usize::from(i == 1), "look {i}");
    }
}

/// Known limitation: a wait without a card has no identity of its own, so
/// one that flaps through running and comes back with another observed
/// time is told again.
#[test]
fn a_wait_without_a_card_that_flaps_through_running_is_told_again() {
    let mut tracker = started();
    let card = waiting("s1", &[("card-a", "permission")]);
    assert_eq!(
        look(&mut tracker, &no_events(), &[card], &on(), T + 1).len(),
        1
    );
    look(&mut tracker, &no_events(), &[running("s1")], &on(), T + 2);
    let mut wait = waiting("s1", &[]);
    wait["liveFacts"]["observedAt"] = iso(T + 3);
    assert_eq!(
        look(&mut tracker, &no_events(), &[wait], &on(), T + 3).len(),
        1
    );
}

#[test]
fn the_real_island_ignores_completions_before_launch_and_off_then_on_replays_nothing() {
    let now = Utc::now().timestamp_millis();
    let mut island = crate::island_events::Tracker::new();
    let mut sound = Tracker::new();
    let mut session = running("s1");
    session["liveFacts"]["outcome"] = json!("completed");
    session["liveFacts"]["turnId"] = json!("old");
    session["liveFacts"]["endedAt"] = iso(now - 1000);
    let events = island.update(&[session.clone()], &[], &[], &[], now);
    assert_eq!(events["items"], json!([]));
    assert_eq!(sound.update(&events, &[], &on(), now)["items"], json!([]));
    session["liveFacts"]["turnId"] = json!("new");
    session["liveFacts"]["endedAt"] = iso(now + 1000);
    let events = island.update(&[session], &[], &[], &[], now + 1001);
    assert_eq!(events["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        sound.update(&events, &[], &off(), now + 1001)["items"],
        json!([])
    );
    assert_eq!(
        sound.update(&events, &[], &on(), now + 1002)["items"],
        json!([])
    );
}

#[test]
fn decided_identities_are_forgotten_a_day_after_last_seen() {
    let mut tracker = started();
    for i in 1..=100 {
        let mut session = waiting("s1", &[]);
        session["liveFacts"]["observedAt"] = iso(T + i);
        tracker.update(&no_events(), &[session], &on(), T + i);
    }
    assert_eq!(tracker.decided.len(), 100);
    tracker.update(&no_events(), &[], &on(), T + 100 + 86_400_000);
    assert!(tracker.decided.is_empty());
}
