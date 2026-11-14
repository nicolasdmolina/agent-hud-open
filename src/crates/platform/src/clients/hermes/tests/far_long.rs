//! A far machine out of reach for five minutes. Run by hand, since it
//! takes that long:
//! `cargo test -p agent-hud-platform --lib clients::hermes::tests_unix::far_long -- --ignored --nocapture`.
use super::far::links::held_down;
use std::time::Duration;

/// The link is killed while a question waits and the machine is kept out
/// of reach for five minutes: the card is there and marked at every look,
/// and is found waiting, and answered, once the machine is back.
#[test]
#[ignore = "takes five minutes: run by hand"]
fn a_far_machine_held_out_of_reach_for_five_minutes_keeps_its_card_marked() {
    held_down(Duration::from_secs(300));
}
