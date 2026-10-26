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

/// Who the call is for, beyond how to reach them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    /// The key the peer's SSH server must answer with, when the method pins one.
    pub host_key: Option<String>,
}

/// Which of the node's keys a call is made with, for methods that have more than one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Key {
    /// The node's own key: a full shell on the peer.
    #[default]
    Node,
    /// The key that may only send input: the peer runs `dojo-mesh inject` whatever is asked.
    Input,
}

pub struct Call {
    pub peer: Peer,
    pub key: Key,
    /// How to reach the peer.
    pub route: Route,
    /// The single command string the peer's shell runs.
    pub remote: String,
    /// Ask the peer for a terminal (an interactive session); calls that carry data never do.
    pub tty: bool,
    pub input: Input,
    pub output: Output,
    pub timeout: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Exited(i32),
    /// Ended by a signal.
    Killed,
    /// The channel needs a person before it can go on: a page to approve this connection at
    /// (Tailscale asks when a policy requires signing in again), or a step to take on this machine
    /// (unlock the key holder). The text is a URL or a sentence.
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

    /// A step a person must take on this machine before any call can succeed, when the channel can
    /// tell without making one (a locked key holder). Most channels cannot and say nothing.
    fn blocked(&self) -> Option<String> {
        None
    }
}
