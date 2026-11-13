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
/// Test rows may carry what the snapshot hides beside them: `heard`, the
/// transcript requests a card hid, and `calls`, each card's tool call.
type Hidden = (
    Vec<Value>,
    crate::presentation::requests::Heard,
    HashMap<String, String>,
);
fn heard(sessions: &[Value]) -> Hidden {
    let (mut sightings, mut calls) = (HashMap::new(), HashMap::new());
    let rows = sessions
        .iter()
        .map(|session| {
            let mut session = session.clone();
            let key = format!(
                "{}:{}",
                session["source"].as_str().unwrap(),
                session["id"].as_str().unwrap()
            );
            let fields = session.as_object_mut().unwrap();
            if let Some(Value::Array(rows)) = fields.remove("heard") {
                sightings.insert(key, rows);
            }
            if let Some(Value::Object(map)) = fields.remove("calls") {
                calls.extend(
                    map.into_iter()
                        .map(|(card, call)| (card, call.as_str().unwrap().to_owned())),
                );
            }
            session
        })
        .collect();
    (rows, sightings, calls)
}
/// A tracker past its first look, which only notes what already waits.
fn started() -> Tracker {
    let mut tracker = Tracker::new();
    tracker.update(&no_events(), &[], &Heard::default(), &on(), T);
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
    let (sessions, sightings, calls) = heard(sessions);
    let heard = Heard {
        sightings: Some(&sightings),
        calls,
    };
    let out = tracker.update(island, &sessions, &heard, settings, now);
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
    let first = tracker.update(&no_events(), &sessions, &Heard::default(), &on(), T + 6);
    assert_eq!(
        tracker.update(&no_events(), &sessions, &Heard::default(), &on(), T + 6),
        first
    );
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
fn a_card_seen_beside_its_sighting_is_one_request() {
    let mut tracker = started();
    let seen = [waiting("s1", &[("log:claude:s1:call-1", "permission")])];
    // Presentation hides the sighting behind the card and notes it.
    let card = [same_as(
        waiting("s1", &[("card-1", "permission")]),
        "log:claude:s1:call-1",
    )];
    assert_eq!(
        look(&mut tracker, &no_events(), &seen, &on(), T + 1).len(),
        1
    );
    assert!(look(&mut tracker, &no_events(), &card, &on(), T + 2).is_empty());
    // The card runs out and the transcript shows the call waiting still.
    assert!(look(&mut tracker, &no_events(), &seen, &on(), T + 3).is_empty());
    // Card first, its sighting read later: still one request.
    let mut tracker = started();
    let alone = [waiting("s1", &[("card-1", "permission")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &alone, &on(), T + 1).len(),
        1
    );
    assert!(look(&mut tracker, &no_events(), &card, &on(), T + 2).is_empty());
    assert!(look(&mut tracker, &no_events(), &seen, &on(), T + 3).is_empty());
}

#[test]
fn a_card_and_a_sighting_never_seen_together_are_two_requests() {
    // Preferring an extra sound to a silenced question.
    let mut tracker = started();
    let seen = [waiting("s1", &[("log:claude:s1:call-1", "permission")])];
    let card = [waiting("s1", &[("card-1", "permission")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &seen, &on(), T + 1).len(),
        1
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &card, &on(), T + 2),
        [needs_you("s1")]
    );
    let other = [waiting("s1", &[("log:claude:s1:call-2", "permission")])];
    assert_eq!(
        look(&mut tracker, &no_events(), &other, &on(), T + 3),
        [needs_you("s1")]
    );
}

#[test]
fn a_card_and_a_sighting_naming_one_call_are_one_request() {
    let mut tracker = started();
    let seen = waiting("s1", &[("log:claude:s1:toolu_1", "permission")]);
    // Never there together: the call they both name makes them one.
    let mut card = waiting("s1", &[("hook:claude:9:1", "permission")]);
    card["calls"] = json!({"hook:claude:9:1":"toolu_1"});
    assert_eq!(
        look(&mut tracker, &no_events(), &[seen], &on(), T + 1).len(),
        1
    );
    assert!(look(&mut tracker, &no_events(), &[card], &on(), T + 2).is_empty());
    let mut other = waiting("s1", &[("hook:claude:9:2", "permission")]);
    other["calls"] = json!({"hook:claude:9:2":"toolu_2"});
    assert_eq!(
        look(&mut tracker, &no_events(), &[other], &on(), T + 3),
        [needs_you("s1")]
    );
}

#[test]
fn a_card_naming_another_call_than_the_sighting_beside_it_sounds() {
    let mut tracker = started();
    let seen = waiting("s1", &[("log:claude:s1:toolu_1", "permission")]);
    assert_eq!(
        look(&mut tracker, &no_events(), &[seen], &on(), T + 1).len(),
        1
    );
    let mut card = same_as(
        waiting("s1", &[("hook:claude:9:2", "permission")]),
        "log:claude:s1:toolu_1",
    );
    card["calls"] = json!({"hook:claude:9:2":"toolu_2"});
    assert_eq!(
        look(&mut tracker, &no_events(), &[card], &on(), T + 2),
        [needs_you("s1")]
    );
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
    let out = tracker.update(&island, &sessions, &Heard::default(), &off(), T + 1);
    assert_eq!(out["items"], json!([]));
    // Turned on while that request still waits and the completion is still
    // in the island's history: neither sounds late.
    assert!(look(&mut tracker, &island, &sessions, &on(), T + 2).is_empty());
    // A setting with no switch at all is off.
    let out = tracker.update(
        &completed("s3", "turn-2"),
        &[],
        &Heard::default(),
        &json!({}),
        T + 3,
    );
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

/// `session`, its one request a card, with the transcript sighting `log`
/// of the same kind there beside it and hidden by it, as presentation does.
fn same_as(mut session: Value, log: &str) -> Value {
    let kind = session["liveFacts"]["requests"][0]["kind"].clone();
    session["heard"] = json!([{"id":log,"sessionId":session["id"],"kind":kind}]);
    session
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
    let out = tracker.update(&no_events(), &sessions, &Heard::default(), &on(), T + 1);
    assert_eq!(out["revision"], 17);
    assert_eq!(out["items"].as_array().unwrap().len(), 17);
    // Forty in one look are all kept too; the next look keeps the island's
    // count of earlier ones beside its own.
    let sessions: Vec<_> = (0..40)
        .map(|i| waiting(&format!("t{i}"), &[("card", "permission")]))
        .collect();
    let out = tracker.update(&no_events(), &sessions, &Heard::default(), &on(), T + 2);
    assert_eq!(out["items"].as_array().unwrap().len(), 57);
    let out = tracker.update(
        &no_events(),
        &[waiting("u", &[("card", "question")])],
        &Heard::default(),
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
    assert_eq!(
        sound.update(&events, &[], &Heard::default(), &on(), now)["items"],
        json!([])
    );
    session["liveFacts"]["turnId"] = json!("new");
    session["liveFacts"]["endedAt"] = iso(now + 1000);
    let events = island.update(&[session], &[], &[], &[], now + 1001);
    assert_eq!(events["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        sound.update(&events, &[], &Heard::default(), &off(), now + 1001)["items"],
        json!([])
    );
    assert_eq!(
        sound.update(&events, &[], &Heard::default(), &on(), now + 1002)["items"],
        json!([])
    );
}

#[test]
fn decided_identities_are_forgotten_a_day_after_last_seen() {
    let mut tracker = started();
    for i in 1..=100 {
        let mut session = waiting("s1", &[]);
        session["liveFacts"]["observedAt"] = iso(T + i);
        tracker.update(&no_events(), &[session], &Heard::default(), &on(), T + i);
    }
    assert_eq!(tracker.decided.len(), 100);
    tracker.update(
        &no_events(),
        &[],
        &Heard::default(),
        &on(),
        T + 100 + 86_400_000,
    );
    assert!(tracker.decided.is_empty());
}

#[test]
fn a_new_question_card_soon_after_an_answered_sighting_sounds() {
    let mut tracker = started();
    let a = waiting_since(
        "s1",
        "log:claude:s1:call-a",
        "question",
        iso(T).as_str().unwrap(),
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &[a], &on(), T + 1).len(),
        1
    );
    // A is answered outside the HUD and a different question B opens five
    // seconds later; no look sees the session running in between. B's card
    // stands for no sighting, as A's was gone when it came.
    let b = waiting_since(
        "s1",
        "hook:claude:epoch:2",
        "question",
        iso(T + 5_000).as_str().unwrap(),
    );
    assert_eq!(
        look(
            &mut tracker,
            &no_events(),
            std::slice::from_ref(&b),
            &on(),
            T + 5_001
        ),
        [needs_you("s1")]
    );
    assert!(look(&mut tracker, &no_events(), &[b], &on(), T + 6_000).is_empty());
}

#[test]
fn the_same_question_carded_eleven_seconds_late_does_not_repeat() {
    let mut tracker = started();
    let a = waiting_since(
        "s1",
        "log:claude:s1:call-a",
        "question",
        iso(T).as_str().unwrap(),
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &[a], &on(), T + 1).len(),
        1
    );
    // The hook for the same call registers eleven seconds later, while the
    // transcript still shows the call open beside it.
    let card = waiting_since(
        "s1",
        "hook:claude:epoch:1",
        "question",
        iso(T + 11_000).as_str().unwrap(),
    );
    let card = same_as(card, "log:claude:s1:call-a");
    assert!(look(&mut tracker, &no_events(), &[card], &on(), T + 11_001).is_empty());
}

#[test]
fn the_same_question_observed_late_does_not_repeat() {
    let mut tracker = started();
    let a = waiting_since(
        "s1",
        "log:claude:s1:call-a",
        "question",
        iso(T).as_str().unwrap(),
    );
    assert_eq!(
        look(&mut tracker, &no_events(), &[a], &on(), T + 1).len(),
        1
    );
    let card = waiting_since(
        "s1",
        "hook:claude:epoch:1",
        "question",
        iso(T + 1_000).as_str().unwrap(),
    );
    let card = same_as(card, "log:claude:s1:call-a");
    assert!(look(&mut tracker, &no_events(), &[card], &on(), T + 11_001).is_empty());
}

#[test]
fn the_ring_keeps_each_look_and_32_before_it_without_growing() {
    let mut tracker = started();
    for pass in 1..=100 {
        let sessions: Vec<_> = (0..100)
            .map(|i| waiting(&format!("s{pass}-{i}"), &[("card", "question")]))
            .collect();
        let out = tracker.update(&no_events(), &sessions, &Heard::default(), &on(), T + pass);
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), if pass == 1 { 100 } else { 132 });
        for session in &sessions {
            assert!(items.iter().any(|a| a["sessionId"] == session["id"]));
        }
    }
    let out = tracker.update(&no_events(), &[], &Heard::default(), &on(), T + 101);
    assert_eq!(out["items"].as_array().unwrap().len(), 32);
}

#[test]
fn a_new_sighting_beside_a_card_of_its_kind_sounds() {
    // Verifier round 3, finding 1: card A waits, then a new question B shows
    // in the transcript; presentation hides both sightings behind A's card.
    let mut tracker = started();
    let card = waiting("s1", &[("hook:1", "question")]);
    let mut first = card.clone();
    first["heard"] = json!([{"id":"log:claude:s1:a","kind":"question"}]);
    assert_eq!(
        look(&mut tracker, &no_events(), &[first], &on(), T + 1).len(),
        1
    );
    let mut both = card;
    both["heard"] = json!([
        {"id":"log:claude:s1:a","kind":"question"},
        {"id":"log:claude:s1:b","kind":"question"}
    ]);
    assert_eq!(
        look(&mut tracker, &no_events(), &[both.clone()], &on(), T + 2),
        [needs_you("s1")]
    );
    assert!(look(&mut tracker, &no_events(), &[both], &on(), T + 3).is_empty());
}

/// Accepted residual (verifier round 3, finding 3): a stale sighting of an
/// answered question still there when a different card naming no call comes
/// is taken for that card, so the card does not sound, nor once the
/// transcript catches up.
#[test]
fn a_stale_sighting_beside_a_distinct_unnamed_card_silences_it() {
    let mut tracker = started();
    let a = waiting("s1", &[("log:claude:s1:a", "question")]);
    assert_eq!(
        look(&mut tracker, &no_events(), &[a], &on(), T + 1).len(),
        1
    );
    let card = || waiting("s1", &[("hook:claude:epoch:2", "question")]);
    let b = same_as(card(), "log:claude:s1:a");
    assert!(look(&mut tracker, &no_events(), &[b], &on(), T + 2).is_empty());
    let caught_up = same_as(card(), "log:claude:s1:b");
    assert!(look(&mut tracker, &no_events(), &[caught_up], &on(), T + 3).is_empty());
}

#[test]
fn a_named_card_hides_every_sighting_of_its_kind_as_upstream() {
    // Verifier round 3, finding 2: presentation shows what upstream shows.
    use crate::presentation::requests::merge;
    let log = |call: &str| json!({"id":format!("log:claude:s1:{call}"),"sessionId":"s1","kind":"question"});
    let card = json!({"id":"hook:1","sessionId":"s1","kind":"question"});
    assert_eq!(
        merge(vec![log("a"), log("b")], std::slice::from_ref(&card)),
        [card]
    );
}

#[test]
fn a_subagent_finishing_inside_its_parents_turn_makes_no_sound() {
    let mut tracker = started();
    // The parent's turn and its subagent's, which carries the parent's turn
    // id, both end; the island tells both.
    let island = json!({"revision":2,"items":[
        {"id":"event:1:a","kind":"completion","source":"claude","sessionId":"agent-a745","turnId":"turn-1"},
        {"id":"event:2:p","kind":"completion","source":"claude","sessionId":"parent","turnId":"turn-1"}
    ]});
    let mut child = running("agent-a745");
    child["parentId"] = json!("parent");
    let mut parent = running("parent");
    parent["parentId"] = Value::Null;
    let alerts = look(
        &mut tracker,
        &island,
        &[child.clone(), parent.clone()],
        &on(),
        T + 1,
    );
    assert_eq!(
        alerts,
        [("finished".into(), "parent".into(), "Hero".into())]
    );
    // Nor later, once the subagent's row has left the list.
    assert!(look(&mut tracker, &island, &[parent], &on(), T + 2).is_empty());
}

/// Verifier round 5: a subagent its log names no parent for (Codex review).
#[test]
fn verifier_codex_review_subagent_without_parent_must_be_silent() {
    let now = Utc::now().timestamp_millis();
    let mut island = crate::island_events::Tracker::new();
    let mut sound = Tracker::new();
    sound.update(&json!({}), &[], &Heard::default(), &on(), now);
    let mut log = agent_hud_agents::Log::new("rollout-review.jsonl".into(), "codex");
    log.parse(&json!({"timestamp":iso(now+1),"type":"session_meta","payload":{"id":"review-child","source":{"subagent":"review"},"cwd":"/work"}}));
    log.parse(&json!({"timestamp":iso(now+2),"type":"event_msg","payload":{"type":"task_started","turn_id":"review-turn"}}));
    log.parse(&json!({"timestamp":iso(now+3),"type":"event_msg","payload":{"type":"task_complete","turn_id":"review-turn","last_agent_message":"Review complete"}}));
    assert!(log.live.is_subagent);
    assert!(!log.live.is_internal);
    let mut row = crate::sessions::log_session(&log, now + 4, None);
    row["liveFacts"] = log.live.facts(&log.id, "codex", now + 4, None).unwrap();
    assert!(row["parentId"].is_null());
    assert_eq!(row["liveFacts"]["outcome"], "completed");
    let sessions = [row];
    let events = island.update(&sessions, &[], &[], &[], now + 4);
    assert_eq!(events["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        sound.update(&events, &sessions, &Heard::default(), &on(), now + 4)["revision"],
        0,
        "recognized Codex review subagent ending is a step, not top-level finished"
    );
}
