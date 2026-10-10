use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn main() {
    if let Err(error) = run() {
        eprintln!("Dojo browser bridge: {error:#}");
        std::process::exit(1);
    }
}

fn read_message(input: &mut impl Read) -> anyhow::Result<Option<Value>> {
    let mut header = [0; 4];
    match input.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let size = u32::from_ne_bytes(header) as usize;
    anyhow::ensure!(size <= 16_384, "native message exceeds size limit");
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn write_message(output: &mut impl Write, value: &Value) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let length = u32::try_from(bytes.len())?;
    output.write_all(&length.to_ne_bytes())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}

fn run() -> anyhow::Result<()> {
    let paths = dojo::theme::Paths::user_default()?;
    let theme_path = paths.root.join("current/zen.json");
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
    let (sender, receiver) = mpsc::sync_channel(32);
    std::thread::Builder::new()
        .name("dojo-browser-input".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match read_message(&mut input) {
                    Ok(Some(message)) => {
                        if sender.send(Ok(message)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(error) => {
                        if let Err(send_error) = sender.send(Err(error)) {
                            eprintln!("Browser input ended: {send_error}");
                        }
                        return;
                    }
                }
            }
        })?;
    let mut previous: Option<String> = None;
    let mut output = io::stdout().lock();
    loop {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(message)) => {
                if let Some(digest) = previous.as_deref() {
                    if message.get("ack").and_then(Value::as_str) == Some(digest) {
                        let ack = runtime.join("dojo-zen-ack.json");
                        let temporary =
                            runtime.join(format!(".dojo-zen-ack-{}.tmp", std::process::id()));
                        let bytes = serde_json::to_vec(
                            &serde_json::json!({"digest":digest,"time":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()}),
                        )?;
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&temporary)?;
                        file.write_all(&bytes)?;
                        file.sync_all()?;
                        fs::rename(temporary, ack)?;
                    }
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let bytes = match fs::read(&theme_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
            bytes.len() <= 128 * 1024,
            "browser theme exceeds size limit"
        );
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if previous.as_ref() == Some(&digest) {
            continue;
        }
        let theme: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            theme.get("colors").and_then(Value::as_object).is_some(),
            "missing browser theme colors"
        );
        write_message(
            &mut output,
            &serde_json::json!({"theme":theme,"digest":digest}),
        )?;
        previous = Some(digest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_frames_round_trip_and_reject_oversized_input() -> anyhow::Result<()> {
        let message = serde_json::json!({"watch":true});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message)?;
        assert_eq!(read_message(&mut bytes.as_slice())?, Some(message));
        assert!(read_message(&mut 20_000_u32.to_ne_bytes().as_slice()).is_err());
        Ok(())
    }
}
