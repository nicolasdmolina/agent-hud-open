//! One WebSocket to the gateway (`hermes-gateway-v1`), opened with a ticket
//! that is used once and dropped. JSON-RPC frames go out and come back;
//! every wait has an end. The ticket rides the subprotocol header and is
//! never written anywhere.
use agent_hud_agents::hermes::gateway::{
    frame, refusal, Command, Failure, Frame, Instance, Refusal, Ticket, SOCKET_PATH, SUBPROTOCOL,
};
use serde_json::Value;
use std::{
    io::ErrorKind,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use tungstenite::{
    client::IntoClientRequest,
    handshake::HandshakeError,
    http::{header::SEC_WEBSOCKET_PROTOCOL, HeaderValue, StatusCode},
    protocol::WebSocketConfig,
    Error, Message, WebSocket,
};

const DIAL: Duration = Duration::from_secs(3);
const HANDSHAKE: Duration = Duration::from_secs(5);
const WRITE: Duration = Duration::from_secs(5);
/// The largest frame taken. `session.resume` answers with a session's whole
/// history, and answering needs it: Hermes's own client stops at 8 MiB,
/// which a long session would pass and so never be answerable.
const FRAME_LIMIT: usize = 64 * 1024 * 1024;
/// A socket that closed without saying why.
const ABNORMAL: u16 = 1006;

pub(super) struct Socket {
    ws: WebSocket<TcpStream>,
    calls: u64,
}

/// What a call came back with.
pub(super) enum Reply {
    /// The whole reply frame.
    Frame(Value),
    /// Nothing in time; the socket still stands.
    Late,
}

impl Socket {
    /// Dials the instance and upgrades with the ticket. No `Origin` header
    /// and no proxy: the address is this machine's, dialled as it is.
    pub(super) fn open(instance: &Instance, ticket: Ticket) -> Result<Self, Failure> {
        let (scheme, authority) = instance
            .origin
            .split_once("://")
            .ok_or(Failure::Invalid("invalid_api_origin"))?;
        if scheme != "http" {
            // A gateway on this machine serves plain HTTP; this build has
            // no TLS to dial another kind with.
            return Err(Failure::Refused(
                "the gateway's origin is not plain HTTP".into(),
            ));
        }
        let address = authority
            .to_socket_addrs()
            .ok()
            .and_then(|mut found| found.find(|address| address.ip().is_loopback()))
            .ok_or(Failure::Invalid("invalid_api_origin"))?;
        let tcp = TcpStream::connect_timeout(&address, DIAL).map_err(|_| Failure::Unreachable)?;
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_read_timeout(Some(HANDSHAKE));
        let _ = tcp.set_write_timeout(Some(WRITE));
        let mut request = format!("ws://{authority}{SOCKET_PATH}")
            .into_client_request()
            .map_err(|_| Failure::Invalid("invalid_api_origin"))?;
        let offered = HeaderValue::from_str(&ticket.subprotocols().join(", "))
            .map_err(|_| Failure::Refused("the ticket cannot be sent".into()))?;
        drop(ticket);
        request
            .headers_mut()
            .insert(SEC_WEBSOCKET_PROTOCOL, offered);
        let config = WebSocketConfig::default()
            .max_message_size(Some(FRAME_LIMIT))
            .max_frame_size(Some(FRAME_LIMIT));
        let (ws, response) =
            match tungstenite::client::client_with_config(request, tcp, Some(config)) {
                Ok(opened) => opened,
                // A ticket opens one socket within 30 s; any other gets 403.
                Err(HandshakeError::Failure(Error::Http(response)))
                    if response.status() == StatusCode::FORBIDDEN =>
                {
                    return Err(Failure::TicketRefused)
                }
                Err(_) => return Err(Failure::Unreachable),
            };
        let spoken = response
            .headers()
            .get(SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok());
        if spoken != Some(SUBPROTOCOL) {
            return Err(Failure::Refused(
                "the gateway speaks another protocol".into(),
            ));
        }
        Ok(Self { ws, calls: 0 })
    }

    /// Sends a call and returns the id its reply will carry.
    pub(super) fn send(&mut self, command: &Command) -> Result<u64, Failure> {
        self.calls += 1;
        self.write(&command.frame(self.calls))?;
        Ok(self.calls)
    }

    /// Writes a frame as it is.
    pub(super) fn write(&mut self, frame: &Value) -> Result<(), Failure> {
        self.ws
            .send(Message::Text(frame.to_string().into()))
            .map_err(|_| Failure::Closed(ABNORMAL))
    }

    /// The next frame the gateway sends, or None when `until` comes first.
    pub(super) fn read(&mut self, until: Instant) -> Result<Option<Value>, Failure> {
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let _ = self.ws.get_ref().set_read_timeout(Some(left));
            let text = match self.ws.read() {
                Ok(Message::Text(text)) => serde_json::from_str::<Value>(text.as_ref()),
                Ok(Message::Binary(bytes)) => serde_json::from_slice::<Value>(&bytes),
                Ok(Message::Close(told)) => {
                    return Err(Failure::Closed(
                        told.map_or(ABNORMAL, |told| u16::from(told.code)),
                    ))
                }
                // A ping is answered by the next write or flush.
                Ok(_) => {
                    let _ = self.ws.flush();
                    continue;
                }
                Err(Error::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    continue
                }
                Err(_) => return Err(Failure::Closed(ABNORMAL)),
            };
            if let Ok(value) = text {
                return Ok(Some(value));
            }
        }
    }

    /// Makes one call and waits for its reply, passing every other frame
    /// that arrives meanwhile to `other`.
    pub(super) fn call(
        &mut self,
        command: &Command,
        wait: Duration,
        other: &mut dyn FnMut(&Value),
    ) -> Result<Reply, Failure> {
        let id = self.send(command)?;
        self.reply(id, wait, other)
    }

    /// Waits for the reply to the call sent with `id`.
    pub(super) fn reply(
        &mut self,
        id: u64,
        wait: Duration,
        other: &mut dyn FnMut(&Value),
    ) -> Result<Reply, Failure> {
        let until = Instant::now() + wait;
        while let Some(value) = self.read(until)? {
            match frame(&value) {
                Frame::Reply { id: of } if of == id => return Ok(Reply::Frame(value)),
                _ => other(&value),
            }
        }
        Ok(Reply::Late)
    }

    /// Closes the socket, which ends whatever it subscribed to. The gateway
    /// is told, not waited for.
    pub(super) fn close(mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

/// The `result` of a reply, or why there is none.
pub(super) fn result(reply: &Value) -> Result<&Value, Refusal> {
    match refusal(reply) {
        Some(refused) => Err(refused),
        None if reply["result"].is_object() => Ok(&reply["result"]),
        None => Err(Refusal::Other("no result".into())),
    }
}
