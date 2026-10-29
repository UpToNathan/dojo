//! Data connections: `GET /api/data` with `rex.data.terminal.v1`. A HELLO (or RESUME) opens the
//! stream; after the WELCOME every binary message is stream bytes.
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use base64::Engine as _;
use tungstenite::{Message as WsMessage, WebSocket};

pub const SUBPROTOCOL: &str = "rex.data.terminal.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Welcome {
    pub reconnect_token: [u8; 16],
    pub resume_window_ms: u32,
    pub nonce: u64,
}

pub fn hello(ticket: &[u8; 16]) -> Vec<u8> {
    let mut message = vec![0x01];
    message.extend_from_slice(ticket);
    message
}

pub fn resume(token: &[u8; 16], received: u64) -> Vec<u8> {
    let mut message = vec![0x02];
    message.extend_from_slice(token);
    message.extend_from_slice(&received.to_le_bytes());
    message
}

/// Reads a WELCOME, or explains a REJECT.
pub fn parse_answer(message: &[u8]) -> Result<Welcome> {
    match message {
        [0x81, rest @ ..] if message.len() == 32 => {
            let token: [u8; 16] = rest[..16].try_into().unwrap();
            let window = u32::from_le_bytes(rest[19..23].try_into().unwrap());
            let nonce = u64::from_le_bytes(rest[23..31].try_into().unwrap());
            Ok(Welcome { reconnect_token: token, resume_window_ms: window, nonce })
        }
        [0x82, reason, ..] => bail!("data connection rejected: {}", reject_reason(*reason)),
        _ => bail!("malformed data handshake answer ({} bytes)", message.len()),
    }
}

fn reject_reason(reason: u8) -> &'static str {
    match reason {
        0x01 => "unknown, expired or used ticket",
        0x02 => "this client already streams that block",
        0x03 => "the stream is not available",
        0x04 => "a limit was reached",
        0x05 => "malformed or late handshake",
        0x06 => "the server failed",
        0x07 => "wrong subprotocol",
        _ => "unknown reason",
    }
}

pub fn decode_ticket(ticket: &str) -> Result<[u8; 16]> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(ticket.trim_end_matches('='))?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("a ticket is 16 bytes"))
}

/// One terminal stream. It counts the bytes handed out, so a dropped connection can resume.
pub struct DataStream {
    socket: PathBuf,
    ws: WebSocket<UnixStream>,
    pub welcome: Welcome,
    received: u64,
}

pub enum Chunk {
    Bytes(Vec<u8>),
    /// The stream ended (status 1000), or the block closed (4000).
    End,
}

impl DataStream {
    pub fn open(socket: &Path, ticket: &[u8; 16]) -> Result<Self> {
        let mut ws = crate::ws::open(socket, "/api/data", SUBPROTOCOL)?;
        ws.send(WsMessage::binary(hello(ticket)))?;
        let welcome = read_answer(&mut ws)?;
        Ok(Self { socket: socket.to_owned(), ws, welcome, received: 0 })
    }

    /// Opens a new connection that continues this stream after the last byte returned.
    pub fn reconnect(&mut self) -> Result<()> {
        let mut ws = crate::ws::open(&self.socket, "/api/data", SUBPROTOCOL)?;
        ws.send(WsMessage::binary(resume(&self.welcome.reconnect_token, self.received)))?;
        let welcome = read_answer(&mut ws)?;
        if welcome.nonce != self.welcome.nonce {
            bail!("resumed stream has a different nonce");
        }
        self.welcome = welcome;
        self.ws = ws;
        Ok(())
    }

    /// A handle that ends the stream from another thread: shutting it down makes a blocked
    /// `next` return an error.
    pub fn shutdown_handle(&self) -> Result<UnixStream> {
        Ok(self.ws.get_ref().try_clone()?)
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    /// The next stream bytes, blocking. An error means the connection dropped; try `reconnect`.
    pub fn next(&mut self) -> Result<Chunk> {
        loop {
            match self.ws.read()? {
                WsMessage::Binary(bytes) => {
                    self.received += bytes.len() as u64;
                    return Ok(Chunk::Bytes(bytes.to_vec()));
                }
                WsMessage::Close(frame) => {
                    let code = frame.as_ref().map(|frame| u16::from(frame.code));
                    if matches!(code, Some(1000) | Some(4000)) {
                        return Ok(Chunk::End);
                    }
                    bail!("data connection closed: {frame:?}");
                }
                _ => {}
            }
        }
    }

    /// Ends the stream on the server (status 1000).
    pub fn close(mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

fn read_answer(ws: &mut WebSocket<UnixStream>) -> Result<Welcome> {
    loop {
        match ws.read()? {
            WsMessage::Binary(bytes) => return parse_answer(&bytes),
            WsMessage::Close(frame) => bail!("data connection closed during handshake: {frame:?}"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_messages_have_the_documented_layout() {
        let ticket = [7u8; 16];
        let hello = hello(&ticket);
        assert_eq!(hello.len(), 17);
        assert_eq!(hello[0], 0x01);
        let resume = resume(&ticket, 0x0102);
        assert_eq!(resume.len(), 25);
        assert_eq!(resume[0], 0x02);
        assert_eq!(&resume[17..], &0x0102u64.to_le_bytes());
    }

    #[test]
    fn a_welcome_carries_token_window_and_nonce() {
        let mut message = vec![0x81];
        message.extend_from_slice(&[9u8; 16]);
        message.extend_from_slice(&[0, 0, 0]);
        message.extend_from_slice(&60_000u32.to_le_bytes());
        message.extend_from_slice(&42u64.to_le_bytes());
        let welcome = parse_answer(&message).unwrap();
        assert_eq!(welcome, Welcome { reconnect_token: [9; 16], resume_window_ms: 60_000, nonce: 42 });
        let reject = parse_answer(&[0x82, 0x07]).unwrap_err().to_string();
        assert!(reject.contains("wrong subprotocol"), "{reject}");
    }

    #[test]
    fn tickets_are_base64url_without_padding() {
        let ticket = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3u8; 16]);
        assert_eq!(ticket.len(), 22);
        assert_eq!(decode_ticket(&ticket).unwrap(), [3; 16]);
    }
}
