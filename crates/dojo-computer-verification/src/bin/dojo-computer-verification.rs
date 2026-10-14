use dojo_computer_verification::steps::{StepEvent, StepLimits, StepRun};
use serde::Deserialize;
use std::io::{Read, Write};
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    StepsNew {
        run_id: String,
        limits: StepLimits,
        now_ms: u64,
    },
    StepsTransition {
        state: StepRun,
        event: StepEvent,
        now_ms: u64,
    },
}
fn run() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin().take(65537).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 65536, "step input exceeds64KiB");
    let command: Command =
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid step command"))?;
    let state = match command {
        Command::StepsNew {
            run_id,
            limits,
            now_ms,
        } => dojo_computer_verification::steps::new_run(run_id, limits, now_ms)?,
        Command::StepsTransition {
            state,
            event,
            now_ms,
        } => dojo_computer_verification::steps::transition(state, event, now_ms)?,
    };
    serde_json::to_writer(std::io::stdout().lock(), &state)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("dojo-computer-verification: {error}");
        std::process::exit(2);
    }
}
