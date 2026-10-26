//! `dojo-mesh shell PEER`: an interactive session on a peer, in a directory, optionally running one
//! known program. A terminal sent to another machine opens this, connected back to the machine it
//! came from.

use crate::channel::{Input, Output};
use crate::cli::{self, Env, Failure, USAGE_ERROR, fail};
use crate::handoff::{self, Program};

/// Changes into the directory (the home directory if it is gone), then runs the program or a login
/// shell. The arguments after the script name are the directory and then the program's words, and
/// reach it as arguments, never as shell text.
const SESSION: &str = "dir=$1; shift; cd \"$dir\" 2>/dev/null || cd \"$HOME\"; if [ $# -eq 0 ]; then exec \"${SHELL:-/bin/sh}\" -l; else exec \"$@\"; fi";

pub fn shell(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let usage = "usage: dojo-mesh shell PEER [--directory DIR] [--program JSON]";
    let (words, values) = cli::split(rest, &[], &["--directory", "--program"])?;
    let [peer] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, usage)) };
    let value = |flag: &str| values.iter().find(|(name, _)| name == flag).map(|(_, text)| text.clone());
    let directory = value("--directory").unwrap_or_else(|| "~".into());
    if directory != "~" && !directory.starts_with('/') {
        return Err(fail(USAGE_ERROR, format!("--directory needs an absolute path, not {directory:?}")));
    }
    let program: Option<Program> = match value("--program") {
        Some(text) => Some(serde_json::from_str(&text).map_err(|error| fail(USAGE_ERROR, format!("--program is not a known program: {error}")))?),
        None => None,
    };
    let mut argv: Vec<String> = ["sh", "-c", SESSION, "dojo-shell"].iter().map(|word| word.to_string()).collect();
    argv.push(if directory == "~" { String::new() } else { directory.clone() });
    if let Some(program) = &program {
        argv.extend(handoff::program_argv(program));
    }
    let config = cli::load_config(env)?;
    let roster = cli::load_roster(env, &config);
    let detail = format!("a session in {directory}");
    let done = cli::call_peer_tty(env, &roster, peer, "shell", &detail, &argv, Input::Inherit, Output::Inherit, None, true)?;
    cli::outcome(peer, &done.ended)
}
