//! Hermes's gateway: the live connection its desktop app uses
//! (`hermes-gateway-v1`), which is the only way to read what a session asks
//! and to answer it. This is the pure half: the shapes, how a frame becomes
//! a card or a change of state, and how a decision becomes the exact frame
//! sent back. Platform holds the sockets (`platform/src/clients/hermes/`),
//! runs the polls `Gateway::poll` begins and does what `Gateway::apply`
//! returns.
//!
//! The hooks stay: they report running and finished as pushes, and they are
//! all there is for a gateway without this connection (`Unsupported`).
mod answering;
mod apart;
mod ask;
mod card;
mod command;
mod connection;
mod decision;
mod effect;
pub mod far;
mod fold;
mod frame;
mod identity;
mod listing;
mod live;
mod machine;
mod poll;
mod quiet;
mod reach;
mod read;
mod reconcile;
mod sample;
mod served;
mod state;
mod ticket;
mod turns;

pub use ask::{Approval, Ask, Batch, BatchQuestion, Choice, Locked, Pending, Prompt, Question};
pub use card::out_of_reach;
pub use command::{active_list, detach, events_since, profiles_list, resume, stats, Command};
pub use connection::{
    identify, refused, served_homes, Connection, Denied, Failure, Identified, Instance, Served,
    Unsupported, SESSION_AUTHORITY,
};
pub use decision::{Answer, NotOffered, Reply};
pub use effect::{Closed, Effect, Input, Status};
pub use frame::{answered, event, frame, hints, refusal, Answered, Cursor, Event, Frame, Refusal};
pub use identity::{
    anchor, hud_session_id, stored_session_id, GatewayId, Profile, ProfileName, SessionRef,
};
pub use listing::{Listed, Listing};
pub use machine::{
    identities_agree, machine_key, reported_install, stored_install, Machine, MachineKey, NoKey,
    StoredKey, INSTALL_ID_FILE,
};
pub use poll::{Poll, Step};
pub use quiet::QUIET_MS;
pub use sample::{Counting, Sample};
pub use state::{Gateway, BUSY_POLL_MS, IDLE_POLL_MS, SNAPSHOT_EVERY_MS};
pub use ticket::{
    granted, identify_line, pointed_socket, ticket_line, Scope, Ticket, CONTROL_POINTER,
    CONTROL_SOCKET, SOCKET_PATH, SUBPROTOCOL,
};
pub use turns::Turn;

#[cfg(test)]
mod tests;
