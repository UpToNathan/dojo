use std::fs::File;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use dojo_speech::{AudioClip, Cancellation, Capture, Selection, Transcriber};

#[derive(Parser)]
#[command(about = "Local speech capture and transcription using Hex's engine")]
struct Args {
    #[arg(long, global = true, default_value = "parakeet_unified_en")]
    model: String,
    #[arg(long, global = true, default_value = "en")]
    language: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the pinned local model catalog and installation status.
    Models,
    /// Keep a transcription model loaded while handling JSON capture commands.
    Serve {
        #[arg(long)]
        device: Option<String>,
    },
    /// List microphones available to the native capture backend.
    Devices,
    /// Download and verify the selected model. Never run implicitly.
    Install,
    /// Transcribe a WAV file and print its text.
    Transcribe { wav: PathBuf },
    /// Record the microphone for a fixed duration, then print its transcript.
    Record {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
        seconds: u64,
        #[arg(long)]
        device: Option<String>,
    },
    /// Capture until stdin receives a newline, then emit JSON transcription events.
    Listen {
        #[arg(long)]
        device: Option<String>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let cancel = Cancellation::default();
    let signal = cancel.clone();
    ctrlc::set_handler(move || signal.cancel())?;
    let selection = Selection::new(&args.model, &args.language)?;
    match args.command {
        Command::Serve { device } => serve(&selection, &cancel, device.as_deref())?,
        Command::Devices => {
            for name in dojo_speech::microphone_devices()? {
                println!("{name}");
            }
        }
        Command::Models => {
            for model in dojo_speech::models()? {
                println!(
                    "{}\t{}\t{} bytes\t{}",
                    model.id,
                    model.name,
                    model.bytes,
                    if model.installed {
                        "installed"
                    } else {
                        "not installed"
                    }
                );
            }
        }
        Command::Install => {
            eprintln!("Installing {}...", selection.model());
            let path = dojo_speech::install_model(&selection, &cancel)?;
            eprintln!("Installed {}", path.display());
        }
        Command::Transcribe { wav } => {
            let clip = AudioClip::from_wav(File::open(wav)?)?;
            let mut transcriber = Transcriber::load(&selection, &cancel)?;
            println!("{}", transcriber.transcribe(clip, &cancel)?);
        }
        Command::Record { seconds, device } => {
            let mut transcriber = Transcriber::load(&selection, &cancel)?;
            if cancel.is_canceled() {
                return Err("canceled".into());
            }
            let capture = Capture::start(device.as_deref())?;
            eprintln!(
                "Recording {} for {seconds} seconds...",
                capture.device_name()
            );
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(seconds) {
                if cancel.is_canceled() {
                    capture.cancel();
                    return Err("canceled".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let clip = capture.finish()?;
            eprintln!("Transcribing on {}...", transcriber.device_name());
            println!("{}", transcriber.transcribe(clip, &cancel)?);
        }
        Command::Listen { device } => {
            let capture = Capture::start(device.as_deref())?;
            println!(
                "{}",
                serde_json::json!({"event": "listening", "device": capture.device_name()})
            );
            std::io::stdout().flush()?;
            let mut stop = String::new();
            if std::io::stdin().lock().read_line(&mut stop)? == 0 || cancel.is_canceled() {
                capture.cancel();
                return Err("capture canceled".into());
            }
            let clip = capture.finish()?;
            println!("{}", serde_json::json!({"event": "transcribing"}));
            std::io::stdout().flush()?;
            let mut transcriber = Transcriber::load(&selection, &cancel)?;
            let text = transcriber.transcribe(clip, &cancel)?;
            println!(
                "{}",
                serde_json::json!({"event": "transcript", "text": text})
            );
        }
    }
    Ok(())
}

fn serve(
    selection: &Selection,
    cancel: &Cancellation,
    device: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut transcriber = Transcriber::load(selection, cancel)?;
    eprintln!("model ready in {} ms", started.elapsed().as_millis());
    emit(serde_json::json!({"event": "ready"}))?;
    let mut capture: Option<(String, Capture)> = None;
    for line in std::io::stdin().lock().lines() {
        let command: serde_json::Value = serde_json::from_str(&line?)?;
        let id = command["id"].as_str().ok_or("missing command id")?;
        let action = command["action"].as_str().ok_or("missing command action")?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            match action {
                "begin" => {
                    if capture.is_some() {
                        return Err("already recording".into());
                    }
                    let input = Capture::start(device)?;
                    emit(
                        serde_json::json!({"event": "listening", "id": id, "device": input.device_name()}),
                    )?;
                    capture = Some((id.to_owned(), input));
                }
                "finish" => {
                    if capture.as_ref().is_some_and(|(active, _)| active == id) {
                        let (_, input) = capture.take().unwrap();
                        let clip = input.finish()?;
                        emit(serde_json::json!({"event": "transcribing", "id": id}))?;
                        let started = Instant::now();
                        let text = transcriber.transcribe(clip, cancel)?;
                        emit(
                            serde_json::json!({"event": "transcript", "id": id, "text": text, "inferenceMs": started.elapsed().as_millis()}),
                        )?;
                    }
                }
                "cancel" => {
                    if capture.as_ref().is_some_and(|(active, _)| active == id) {
                        let (_, input) = capture.take().unwrap();
                        input.cancel();
                    }
                }
                _ => return Err("unknown capture command".into()),
            }
            Ok(())
        })();
        if let Err(error) = result {
            emit(serde_json::json!({"event": "error", "id": id, "message": error.to_string()}))?;
        }
    }
    Ok(())
}

fn emit(event: serde_json::Value) -> std::io::Result<()> {
    println!("{event}");
    std::io::stdout().flush()
}
