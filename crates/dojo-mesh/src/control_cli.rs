//! Placeholder until the control plane lands in the next change.
use crate::cli::{Env, Failure, USAGE_ERROR, fail};

pub fn run(_env: &Env, command: &str, _rest: &[String]) -> Result<u8, Failure> {
    Err(fail(USAGE_ERROR, format!("{command} is not available yet")))
}
