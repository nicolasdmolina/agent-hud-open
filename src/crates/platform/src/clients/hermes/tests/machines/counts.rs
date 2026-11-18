//! A far session's count while it runs (ticket 35), in the lab of
//! `machines.rs`: the machines the app runs, a far gateway behind a
//! throwaway `sshd`, the mirrors' writer with the app's own spacing, and
//! a real core that reads the mirrors' folder with no file watcher.
//!
//! The gateway never gives a hint, so only the polls find a count that
//! moved. What a row shows is what `sessions` answers, read after the
//! turn of the collector that the writer's word asks for.
use super::{
    super::fake::Fake,
    fleet, here,
    mirrors::{far, works},
    ssh, until, Fleet, INSTALL,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// What `session.stats` says of `session` from now on.
pub(in crate::clients::hermes) fn uses(gateway: &Fake, session: &str, input: u64, output: u64) {
    let mut world = gateway.world();
    world.stream_in("default", session).stats = json!({
        "input_tokens": input, "output_tokens": output, "cache_read_tokens": 0,
        "cache_write_tokens": 0, "total_tokens": input + output, "model": "claude-test",
    });
}

impl Fleet {
    /// The tokens the row of the far session `id` shows, after the turn
    /// the writers asked for.
    pub(in crate::clients::hermes) fn tokens(&mut self, id: &str) -> Value {
        self.turn();
        let row = self.far_rows().into_iter().find(|row| row.id == id);
        row.map_or(Value::Null, |row| row.tokens)
    }

    /// Waits until the row of `id` shows `tokens`; how long that took.
    pub(in crate::clients::hermes) fn shows(&mut self, id: &str, tokens: u64) -> Duration {
        let began = Instant::now();
        let mut last = Value::Null;
        until(&format!("{id} shows {tokens}"), || {
            last = self.tokens(id);
            last == json!(tokens)
        });
        began.elapsed()
    }
}

/// How often the gateway was asked `method`.
fn asked(gateway: &Fake, method: &str) -> usize {
    gateway.world().calls(method).len()
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_sessions_count_rises_while_its_turn_runs() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    fleet.reads_mirrors();
    let before = fleet.snapshot();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-turn", "Counts as it goes");
    works(&gateway, "s-job", "A scheduled job");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    let (turn, job) = (far(&key, "s-turn"), far(&key, "s-job"));
    fleet.listed(&turn);
    fleet.listed(&job);
    assert_eq!(fleet.tokens(&turn), json!(0));
    let began = Instant::now();
    let said = |what: &str, took: Duration, fleet: &mut Fleet| {
        let (at, shown) = (began.elapsed().as_millis(), fleet.tokens(&turn));
        println!(
            "COUNT {at:>6} ms: {what}; the row shows {shown} ({} ms later)",
            took.as_millis()
        );
    };

    // The turn's model answers call after call, and the count on the
    // row follows, with nothing told to the HUD but by its own polls.
    for (input, output) in [(100, 20), (260, 40), (900, 100)] {
        uses(&gateway, "s-turn", input, output);
        let took = fleet.shows(&turn, input + output);
        said(
            &format!("Hermes says {input} in, {output} out"),
            took,
            &mut fleet,
        );
    }
    // The turn ends, and its last call added to the count: read once
    // more, though the session works no longer.
    uses(&gateway, "s-turn", 1_200, 150);
    gateway.world().stream_in("default", "s-turn").running = false;
    let took = fleet.shows(&turn, 1_350);
    said("the turn ended at 1200 in, 150 out", took, &mut fleet);
    // Idle now: nothing asks for its count again.
    let stats = asked(&gateway, "session.stats");
    let lists = asked(&gateway, "session.active_list");
    until("four more polls", || {
        asked(&gateway, "session.active_list") >= lists + 4
    });
    let turn_asks = |gateway: &Fake| {
        let of = |(_, frame): &(usize, Value)| frame["params"]["session_id"] == "s-turn";
        gateway
            .world()
            .calls("session.stats")
            .iter()
            .filter(|call| of(call))
            .count()
    };
    let idle_asks = turn_asks(&gateway);
    until("two more polls", || {
        asked(&gateway, "session.active_list") >= lists + 6
    });
    assert_eq!(
        turn_asks(&gateway),
        idle_asks,
        "an idle session is not asked"
    );
    assert!(
        asked(&gateway, "session.stats") > stats,
        "the one that works is"
    );

    // A new turn in the same session, with an agent Hermes loaded anew:
    // it counts from nothing, and the row stays where it was until the
    // count passes it.
    uses(&gateway, "s-turn", 30, 5);
    gateway.world().stream_in("default", "s-turn").running = true;
    let polls = asked(&gateway, "session.active_list");
    until("three polls", || {
        asked(&gateway, "session.active_list") >= polls + 3
    });
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(
        fleet.tokens(&turn),
        json!(1_350),
        "a lower count changes nothing"
    );
    said(
        "a new agent counts 30 in, 5 out",
        Duration::ZERO,
        &mut fleet,
    );
    uses(&gateway, "s-turn", 2_000, 300);
    let took = fleet.shows(&turn, 2_300);
    said(
        "it passed the old count: 2000 in, 300 out",
        took,
        &mut fleet,
    );

    // The job's session works all along, and Hermes holds no agent for
    // it: zeros, which are no count.
    assert_eq!(fleet.tokens(&job), json!(0));
    // Nothing of it was counted: the totals are as they were.
    let after = fleet.snapshot();
    assert_eq!(after["totals"], before["totals"]);
    // And nothing but reads was asked, on the one link.
    let methods: std::collections::BTreeSet<String> = (gateway.world().received.iter())
        .filter_map(|(_, frame)| frame["method"].as_str().map(str::to_owned))
        .collect();
    let reads = [
        "profiles.list",
        "session.active_list",
        "session.events.since",
        "session.stats",
    ];
    assert_eq!(
        methods.iter().map(String::as_str).collect::<Vec<_>>(),
        reads
    );
    assert_eq!(fleet.masters().len(), 1, "{:?}", fleet.masters());
    println!("{}", fleet.log());
}

/// After a relaunch with the far machine out of reach, the row shows
/// the count it showed.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_count_shows_after_a_relaunch_with_the_machine_out_of_reach() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-turn", "Counts as it goes");
    uses(&gateway, "s-turn", 400, 60);
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    let turn = far(&key, "s-turn");
    fleet.listed(&turn);
    fleet.shows(&turn, 460);

    // The app quits, and the far gateway is gone when it is launched
    // again: nothing can be read of the machine.
    fleet.stop();
    drop(gateway);
    fleet.told.lock().unwrap().clear();
    fleet.reads_mirrors();
    assert_eq!(fleet.tokens(&turn), json!(460), "from what was saved");
    fleet.start();
    fleet.looked(6);
    assert_eq!(fleet.tokens(&turn), json!(460));
    let rows = fleet.far_rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].machine, "Mini");
}
