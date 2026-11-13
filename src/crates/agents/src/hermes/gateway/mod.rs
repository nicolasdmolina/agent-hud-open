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
mod ask;
mod card;
mod command;
mod connection;
mod decision;
mod effect;
mod fold;
mod frame;
mod identity;
mod poll;
mod read;
mod reconcile;
mod state;
mod ticket;

pub use ask::{Approval, Ask, Batch, BatchQuestion, Choice, Locked, Pending, Prompt, Question};
pub use command::{active_list, detach, events_since, resume, Command};
pub use connection::{
    identify, refused, Connection, Denied, Failure, Identified, Instance, Served, Unsupported,
    SESSION_AUTHORITY,
};
pub use decision::{Answer, NotOffered, Reply};
pub use effect::{Closed, Effect, Input, Status};
pub use frame::{answered, event, frame, hints, refusal, Answered, Cursor, Event, Frame, Refusal};
pub use identity::{GatewayId, Profile, SessionRef};
pub use poll::{Poll, Step};
pub use state::{Gateway, BUSY_POLL_MS, IDLE_POLL_MS};
pub use ticket::{
    granted, identify_line, pointed_socket, ticket_line, Scope, Ticket, CONTROL_POINTER,
    CONTROL_SOCKET, SOCKET_PATH, SUBPROTOCOL,
};

#[cfg(test)]
mod tests;
