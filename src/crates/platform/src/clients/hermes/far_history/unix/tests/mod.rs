//! The reader against a throwaway `sshd` on loopback (ticket 28's lab,
//! `ssh_link/unix/tests/lab.rs`) and a `state.db` made from Hermes's own
//! schema with made-up rows. "Far" is this machine, reached through that
//! `sshd`, so a test can count the reader's processes and see which of
//! them has the database open: every count here is taken on that side,
//! from the process list and the open files, never from what the caller
//! believes.
//!
//! The tests are marked `#[ignore]` and run by `the_lab_tests_pass` in a
//! process of their own, as the link's are. To run them here and see what
//! each prints:
//! `cargo test -p agent-hud-platform --lib clients::hermes::far_history::tests -- --ignored --nocapture`.
mod deadline;
mod exclusion;
mod idle;
mod reads;
mod refusals;
mod schema;
mod slot;
mod world;

#[test]
fn the_lab_tests_pass() {
    crate::ssh_link::unix::tests::lab::in_a_process_of_their_own(
        "clients::hermes::far_history::tests::",
    );
}
