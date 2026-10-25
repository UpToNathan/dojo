//! The channel seam: running a command, or carrying a file, on a peer. A mesh method supplies the
//! implementation (`tailscale ssh` today); everything above it sees only a call and how it ended.

use crate::method::Route;
use std::fs::File;
use std::process::Stdio;
use std::time::Duration;

pub enum Input {
    Inherit,
    Null,
    File(File),
    /// Written to the call's standard input, which is then closed.
    Bytes(Vec<u8>),
    /// Another process's output, or any stream, as the call's standard input.
    Stdio(Stdio),
}

pub enum Output {
    Inherit,
    /// Collected and returned in [`Done::stdout`].
    Capture,
    File(File),
}

pub struct Call {
    /// How to reach the peer.
    pub route: Route,
    /// The single command string the peer's shell runs.
    pub remote: String,
    pub input: Input,
    pub output: Output,
    pub timeout: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Exited(i32),
    /// Ended by a signal.
    Killed,
    /// The channel wants a person to approve this connection at the given page (Tailscale asks
    /// when a policy requires signing in again).
    NeedsHuman(String),
    TimedOut,
}

#[derive(Debug)]
pub struct Done {
    pub ended: Ended,
    pub stdout: Vec<u8>,
}


/// Carries a call to a peer. `Err` means the call could not be started at all, and says why.
pub trait Channel: Send + Sync {
    fn run(&self, call: Call) -> Result<Done, String>;
}
