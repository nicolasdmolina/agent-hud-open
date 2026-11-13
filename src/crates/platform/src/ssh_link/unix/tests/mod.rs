//! The link against a throwaway `sshd` on loopback (`lab.rs`). No test
//! reads the user's `ssh` configuration, keys, `known_hosts` or agent.
mod cleanup;
pub(crate) mod lab;
mod limits;
mod link;
mod liveness;
mod pure;
mod trust;
