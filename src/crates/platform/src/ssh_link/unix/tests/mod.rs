//! The link against a throwaway `sshd` on loopback (`lab.rs`). No test
//! reads the user's `ssh` configuration, keys, `known_hosts` or agent.
//!
//! The tests that start processes are marked `#[ignore]` and run by
//! `the_lab_tests_pass`, in a process of their own
//! (`lab::in_a_process_of_their_own` says why). To run them here and see
//! what each prints:
//! `cargo test -p agent-hud-platform --lib ssh_link::unix::tests -- --ignored --nocapture`.
mod cleanup;
mod isolation;
pub(crate) mod lab;
mod limits;
mod link;
mod liveness;
mod pure;
mod trust;

#[test]
fn the_lab_tests_pass() {
    lab::in_a_process_of_their_own("ssh_link::unix::tests::");
}
