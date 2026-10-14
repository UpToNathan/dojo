use dojo_computer_recording::{
    Evaluator, MAX_INPUT_BYTES, Recorder, RecordingPolicy, TraceEvent, TrialResult,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    EvaluateGeneral {
        metric: dojo_computer_recording::general_metrics::Metric,
        evidence: Option<Value>,
    },
    Usage {
        samples: Vec<dojo_computer_recording::usage::UsageSample>,
    },
    GuiR1 {
        input: dojo_computer_recording::processors::ProcessorInput,
    },
    Aguvis {
        input: dojo_computer_recording::processors::ProcessorInput,
        seed: u64,
        count: usize,
    },
    Benchmark {
        tasks: Vec<dojo_computer_recording::benchmark::DatasetTask>,
        #[serde(default)]
        limits: dojo_computer_recording::benchmark::EpisodeLimits,
        #[serde(default)]
        policy: RecordingPolicy,
    },
    Record {
        session_id: String,
        #[serde(default)]
        policy: RecordingPolicy,
        events: Vec<TraceEvent>,
        #[serde(default)]
        usage: Option<Vec<dojo_computer_recording::usage::UsageSample>>,
    },
    Analyze {
        trace: Value,
        #[serde(default)]
        policy: RecordingPolicy,
    },
    Atif {
        trace: Value,
        #[serde(default)]
        policy: RecordingPolicy,
        #[serde(default)]
        usage: Option<Vec<dojo_computer_recording::usage::UsageSample>>,
    },
    Transcript {
        cast: String,
        time_ms: u64,
        #[serde(default)]
        secret_values: Vec<String>,
    },
    Evaluate {
        evaluator: Evaluator,
        evidence: Option<Value>,
    },
    Metrics {
        results: Vec<TrialResult>,
        evaluation: Option<Value>,
    },
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin()
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("stdin exceeds 8 MiB".into());
    }
    let command: Command = serde_json::from_slice(&bytes).map_err(|_| "invalid command JSON")?;
    let output = match command {
        Command::EvaluateGeneral { metric, evidence } => serde_json::to_value(
            dojo_computer_recording::general_metrics::evaluate(&metric, evidence.as_ref())?,
        )?,
        Command::Usage { samples } => {
            serde_json::to_value(dojo_computer_recording::usage::aggregate(&samples)?)?
        }
        Command::GuiR1 { input } => {
            serde_json::to_value(dojo_computer_recording::processors::gui_r1(&input)?)?
        }
        Command::Aguvis { input, seed, count } => serde_json::to_value(
            dojo_computer_recording::processors::aguvis(&input, seed, count)?,
        )?,
        Command::Benchmark {
            tasks,
            limits,
            policy,
        } => serde_json::to_value(dojo_computer_recording::benchmark::run_tasks(
            &tasks, &limits, policy,
        )?)?,
        Command::Record {
            session_id,
            policy,
            events,
            usage,
        } => {
            let mut recorder = Recorder::new(&session_id, policy)?;
            for event in events {
                recorder.push(event)?;
            }
            if let Some(samples) = usage {
                recorder.set_usage(&samples)?;
            }
            serde_json::to_value(recorder.finish())?
        }
        Command::Analyze { trace, policy } => {
            serde_json::to_value(dojo_computer_recording::analyze(
                &dojo_computer_recording::ingest(&serde_json::to_vec(&trace)?, policy)?,
            ))?
        }
        Command::Atif {
            trace,
            policy,
            usage,
        } => {
            let mut output = dojo_computer_recording::export_atif(
                &dojo_computer_recording::ingest(&serde_json::to_vec(&trace)?, policy)?,
            );
            if let Some(samples) = usage {
                output["final_metrics"]["extra"]["usage"] =
                    serde_json::to_value(dojo_computer_recording::usage::aggregate(&samples)?)?;
            }
            output
        }
        Command::Transcript {
            cast,
            time_ms,
            secret_values,
        } => dojo_computer_recording::observe_transcript(&cast, time_ms, &secret_values)?,
        Command::Evaluate {
            evaluator,
            evidence,
        } => serde_json::to_value(dojo_computer_recording::evaluate(
            &evaluator,
            evidence.as_ref(),
        ))?,
        Command::Metrics {
            results,
            evaluation,
        } => json!({"pass_at_k": dojo_computer_recording::pass_at_k(&results),
            "reward": evaluation.as_ref().and_then(dojo_computer_recording::reward_of)}),
    };
    serde_json::to_writer(io::stdout().lock(), &output)?;
    println!();
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        // Errors describe structure and limits only; never dump untrusted input.
        eprintln!("dojo-computer-recording: {error}");
        std::process::exit(2);
    }
}
