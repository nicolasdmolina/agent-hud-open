//! The far control relay and discovery, through ticket 28's throwaway
//! `sshd` on loopback (`ssh_link/unix/tests/lab.rs`). "Far" is this same
//! machine, signed in as the user who runs the tests, with a home folder
//! of the lab's own: no test looks at the user's real Hermes home, and
//! none reads the user's `ssh` files.
//!
//! The gateways are pretend ones on real unix sockets (`fixture.rs`), so
//! each can be made to misbehave. `real.rs` runs the same against a real
//! Hermes gateway in a home of its own, when asked for by hand.
//!
//! The tests that start processes are marked `#[ignore]` and run by
//! `the_lab_tests_pass`, in a process of their own. To run them here and
//! see what each prints:
//! `cargo test -p agent-hud-platform --lib clients::hermes::far::tests -- --ignored --nocapture`.
use crate::ssh_link::unix::tests::lab;

mod checks;
mod deadline;
mod discovery;
mod fixture;
mod lists;
mod program;
mod real;
mod replies;
mod secret;

#[test]
fn the_lab_tests_pass() {
    lab::in_a_process_of_their_own("clients::hermes::far::tests::");
}
