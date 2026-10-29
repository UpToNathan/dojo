//! Keeping a libghostty mirror of one terminal current from its data stream.
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::data::{Chunk, DataStream};
use crate::stream::{Demux, Message, Piece};
use crate::terminal::Terminal;

/// What the stream thread shares with the view.
pub struct Shared {
    pub terminal: Mutex<Option<Terminal>>,
    /// The program exited or the block closed: the view should go away.
    pub ended: AtomicBool,
    /// Why the stream stopped, if it failed.
    pub error: Mutex<Option<String>>,
}

pub(crate) fn run_stream(mut stream: DataStream, shared: &Shared, wake: &dyn Fn()) -> Result<()> {
    let (terminal, rest) = Terminal::from_snapshot(&mut || match stream.next()? {
        Chunk::Bytes(bytes) => Ok(Some(bytes)),
        Chunk::End => Ok(None),
    })?;
    *shared.terminal.lock().unwrap() = Some(terminal);
    let mut demux = Demux::new(stream.welcome.nonce);
    let mut pending = rest;
    loop {
        if !pending.is_empty() {
            let mut guard = shared.terminal.lock().unwrap();
            let terminal = guard.as_mut().expect("terminal restored");
            for piece in demux.feed(&pending) {
                match piece {
                    Piece::Bytes(bytes) => terminal.write(&bytes),
                    Piece::Message(Message::ChildExited { .. }) => {
                        drop(guard);
                        return Ok(());
                    }
                    Piece::Message(message) => terminal.apply(&message)?,
                }
            }
            drop(guard);
            wake();
        }
        pending = match next_with_resume(&mut stream) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        };
    }
}

/// The next chunk; a dropped connection resumes while the server still holds the stream.
fn next_with_resume(stream: &mut DataStream) -> Result<Option<Vec<u8>>> {
    let mut failure = match stream.next() {
        Ok(Chunk::Bytes(bytes)) => return Ok(Some(bytes)),
        Ok(Chunk::End) => return Ok(None),
        Err(error) => error,
    };
    let deadline = Instant::now() + Duration::from_millis(stream.welcome.resume_window_ms.min(30_000) as u64);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        match stream.reconnect() {
            Ok(()) => return next_with_resume(stream),
            Err(error) => failure = error,
        }
    }
    Err(anyhow!(failure).context("the data connection dropped and could not resume"))
}
